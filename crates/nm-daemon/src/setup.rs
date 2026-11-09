//! Creating, renaming and removing an IMAP account (`plan-imap.md`, phase
//! 7; `import-initial.md`; `configuration.md`, sections 5 and 6).
//!
//! These commands carry a password, or delete mail: only a human's client
//! (app, CLI, TUI) may call them, never an agent (`configuration.md`,
//! section 6). No lock of the core is held while the server answers.
//!
//! `add_account`, in this order, so that the worker that `accounts.toml`
//! starts finds everything ready:
//! 1. log in; list the folders; create those the roles need;
//! 2. read the boundary of the screening and the recipients of Sent;
//! 3. database: the account, the roles, the boundary, the correspondents'
//!    decisions, the server-side archiving queued;
//! 4. the password in the Keychain, bound to this server;
//! 5. `accounts.toml`, read again and written atomically; then applied at
//!    once (the worker starts, sends the archiving, then synchronizes).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use nm_api::{
    AddAccountParams, AddAccountResult, ArchiveCount, CountArchiveParams, DiscoverAccountParams, DiscoveryReport,
    RemoveAccountParams, RenameAccountParams, RpcError, SetFolderRolesParams, SmtpTestReport, TestAccountParams,
    TestAccountReport, TestSmtpParams, error_code,
};
use nm_core::{FolderRole, Source};
use nm_engine::config_write::{self, NewAccount, NewServer};
use nm_engine::providers::Existing;
use nm_engine::secrets::{Binding, Credential};
use nm_engine::setup;
use nm_store::setup::{ArchiveCriteria, RoleAssignment};

use crate::{Core, with_store};

/// Only a human enters a password or deletes an account.
fn human(source: Source) -> Result<(), RpcError> {
    if matches!(source, Source::App | Source::Cli | Source::Tui) {
        Ok(())
    } else {
        Err(RpcError::new(error_code::FORBIDDEN, "only the app, the TUI and the CLI can do this"))
    }
}

fn server(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(error_code::SERVER, e.to_string())
}

fn invalid(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(error_code::INVALID_PARAMS, e.to_string())
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The account's own addresses, lowercase.
fn own(email: &str, identities: &[String]) -> Vec<String> {
    std::iter::once(email).chain(identities.iter().map(String::as_str)).map(|a| a.trim().to_lowercase()).collect()
}

/// `test_account`: the server tried, nothing written anywhere.
pub async fn test_account(core: &Arc<Core>, p: TestAccountParams, source: Source) -> Result<TestAccountReport, RpcError> {
    human(source)?;
    let spec = setup::spec_of(&p.imap).map_err(invalid)?;
    let user = setup::username_of(&p.imap, &p.email);
    let import = core.engine.lock().unwrap().config().import.clone();
    setup::test_account(&spec, &user, &p.password, &own(&p.email, &p.identities), &import, now()).await.map_err(server)
}

/// `discover_account`: the servers of an address, from the database of
/// providers and the MX of its domain (`docs/accounts.md`, "Finding the
/// servers"). Nothing is written, no mail server is contacted.
pub async fn discover_account(core: &Arc<Core>, p: DiscoverAccountParams, source: Source) -> Result<DiscoveryReport, RpcError> {
    human(source)?;
    if let Some(imap) = &p.imap {
        setup::spec_of(imap).map_err(invalid)?;
    }
    // Every account the core has, for the key proposed: those of the
    // database (IMAP, disabled ones too, and archives), as `list_accounts`
    // gives them, and those of the file the database has not seen yet.
    let rows = crate::with_reader(core, |s| s.list_account_rows()).await?;
    let existing: Vec<Existing> = {
        let engine = core.engine.lock().unwrap();
        let mut v: Vec<Existing> =
            engine.account_views(rows).into_iter().map(|a| Existing { key: a.key, email: a.email }).collect();
        for (key, a) in &engine.config().accounts {
            if !v.iter().any(|e| &e.key == key) {
                v.push(Existing { key: key.clone(), email: Some(a.email.clone()) });
            }
        }
        v
    };
    nm_engine::discover::discover(&p.email, &existing, p.imap.as_ref()).await.map_err(invalid)
}

/// `test_smtp`: the candidates tried in order, the first that works
/// chosen. The IMAP password goes only to a server of the IMAP server's
/// organization (`nm_engine::discover::test_smtp`).
pub async fn test_smtp(p: TestSmtpParams, source: Source) -> Result<SmtpTestReport, RpcError> {
    human(source)?;
    for c in &p.candidates {
        setup::spec_of(&c.server).map_err(invalid)?;
    }
    Ok(nm_engine::discover::test_smtp(&p.email, &p.imap, &p.password, &p.candidates).await)
}

/// The `security` of `accounts.toml` from the client's word, checked.
fn security_of(p: &nm_api::ImapParams) -> Result<nm_engine::config::Security, RpcError> {
    Ok(match setup::spec_of(p).map_err(invalid)?.security {
        nm_proto::imap::Security::Tls => nm_engine::config::Security::Tls,
        nm_proto::imap::Security::Starttls => nm_engine::config::Security::Starttls,
        nm_proto::imap::Security::None => nm_engine::config::Security::None,
    })
}

/// `count_archive`: the exact count of the import screen.
pub async fn count_archive(p: CountArchiveParams, source: Source) -> Result<ArchiveCount, RpcError> {
    human(source)?;
    let spec = setup::spec_of(&p.imap).map_err(invalid)?;
    let user = setup::username_of(&p.imap, &p.email);
    for d in [&p.read_before, &p.unread_before].into_iter().flatten() {
        setup::parse_day(d).ok_or_else(|| invalid(format!("date {d:?}: YYYY-MM-DD expected")))?;
    }
    let mut s = setup::open(&spec, &user, &p.password).await.map_err(server)?;
    s.forbid_writes();
    let n = setup::count_archive(&mut s, p.uidnext, p.read_before.as_deref(), p.unread_before.as_deref()).await.map_err(server)?;
    s.logout().await;
    Ok(n)
}

/// The folder of each workflow role: the user's choice, else the proposal
/// (special-use, then names), else a new folder under the role's English
/// name, in the personal namespace.
fn roles_for(
    chosen: &BTreeMap<FolderRole, String>,
    entries: &[nm_proto::imap::ListEntry],
    namespace: Option<&nm_proto::imap::Namespace>,
) -> BTreeMap<FolderRole, String> {
    let proposed = nm_engine::folders::propose_roles(entries, namespace);
    let mut out = BTreeMap::new();
    for role in FolderRole::ALL {
        let path = if role == FolderRole::Inbox {
            "INBOX".to_string()
        } else if let Some(p) = chosen.get(&role) {
            p.clone()
        } else if let Some((e, _)) =
            entries.iter().zip(&proposed).find(|(_, r)| r.as_ref().is_some_and(|(r, _)| *r == role.as_str()))
        {
            e.name.name.clone()
        } else {
            format!("{}{}", namespace.map(|n| n.prefix.as_str()).unwrap_or(""), role.display_name())
        };
        out.insert(role, path);
    }
    out
}

/// `add_account`.
pub async fn add_account(core: &Arc<Core>, p: AddAccountParams, source: Source) -> Result<AddAccountResult, RpcError> {
    human(source)?;
    config_write::check_key(&p.key).map_err(invalid)?;
    if p.password.is_empty() {
        return Err(invalid("empty password"));
    }
    let (config_path, secrets, import) = {
        let engine = core.engine.lock().unwrap();
        if engine.config().accounts.contains_key(&p.key) {
            return Err(invalid(format!("accounts.toml already has an account {}", p.key)));
        }
        (engine.accounts_path(), engine.secrets(), engine.config().import.clone())
    };
    for d in [&p.archive_read_before, &p.archive_unread_before].into_iter().flatten() {
        setup::parse_day(d).ok_or_else(|| invalid(format!("date {d:?}: YYYY-MM-DD expected")))?;
    }
    if p.read_only && (p.archive_read_before.is_some() || p.archive_unread_before.is_some()) {
        return Err(invalid("a read-only account archives nothing on its server"));
    }
    let spec = setup::spec_of(&p.imap).map_err(invalid)?;
    let user = setup::username_of(&p.imap, &p.email);
    let own = own(&p.email, &[]);
    // Checked before anything is done: a wrong SMTP server must not leave
    // an account half created.
    let smtp = match &p.smtp {
        Some(s) => Some(NewServer {
            host: s.host.clone(),
            port: s.port,
            security: security_of(s)?,
            username: setup::username_of(s, &p.email),
            ca_file: s.ca_file.clone(),
        }),
        None => None,
    };

    // 1 and 2: the server.
    let mut s = setup::open(&spec, &user, &p.password).await.map_err(server)?;
    if p.read_only {
        s.forbid_writes();
    }
    let namespace = s.namespace().await.ok().and_then(|n| n.personal.into_iter().next());
    let entries = s.list().await.map_err(server)?;
    let mut roles = roles_for(&p.roles, &entries, namespace.as_ref());
    let created = if p.read_only {
        // Nothing created: the roles without folder stay unassigned.
        let existing: BTreeSet<String> = entries.iter().map(|e| e.name.name.clone()).collect();
        roles.retain(|r, path| *r == FolderRole::Inbox || existing.contains(path));
        Vec::new()
    } else {
        let paths: Vec<String> = roles.values().cloned().collect();
        setup::create_folders(&mut s, &paths).await.map_err(server)?
    };
    let boundary = match p.uidnext {
        Some(u) => u,
        None => s.status(&nm_proto::imap::MailboxName::inbox()).await.map_err(server)?.uid_next.unwrap_or(1),
    };
    let recipients = if p.correspondents && let Some(sent) = roles.get(&FolderRole::Sent) {
        setup::sent_recipients(&mut s, sent, &own).await.map_err(server)?
    } else {
        Vec::new()
    };
    s.logout().await;

    // 3: the database.
    let delimiter: BTreeMap<String, Option<String>> = entries.iter().map(|e| (e.name.name.clone(), e.delimiter.map(String::from))).collect();
    let default_delim = entries.iter().find_map(|e| e.delimiter).map(String::from);
    let assignments: Vec<RoleAssignment> = roles
        .iter()
        .map(|(role, path)| RoleAssignment {
            path: path.clone(),
            delimiter: delimiter.get(path).cloned().unwrap_or_else(|| default_delim.clone()),
            role: Some(*role),
        })
        .collect();
    let archive_to = roles.get(&FolderRole::Archive).cloned().unwrap_or_default();
    let archive = (p.archive_read_before.is_some() || p.archive_unread_before.is_some()).then(|| ArchiveCriteria {
        from_path: "INBOX".into(),
        to_path: archive_to,
        boundary,
        read_before: p.archive_read_before.clone(),
        unread_before: p.archive_unread_before.clone(),
        batch_size: import.batch_size as u32,
    });
    let key = p.key.clone();
    let own2 = own.clone();
    let (account_id, correspondents) = with_store(core, move |st| {
        let id = st.prepare_imap_account(&key, source)?;
        st.set_folder_roles(id, &assignments, source)?;
        let inbox: i64 = st.conn().query_row("SELECT id FROM folder WHERE account_id = ?1 AND role = 'inbox'", [id], |r| r.get(0))?;
        st.set_screening_start(inbox, boundary)?;
        let n = st.add_correspondents(&recipients, &own2)?;
        if let Some(c) = &archive {
            st.queue_initial_archive(id, c, source)?;
        }
        Ok((id, n))
    })
    .await?;

    // 4: the password, for this server.
    let credential = Credential {
        binding: Binding { host: p.imap.host.clone(), username: user.clone() },
        secret: nm_engine::config::SecretString::new(p.password.clone()),
    };
    let key = p.key.clone();
    let secrets2 = secrets.clone();
    tokio::task::spawn_blocking(move || secrets2.set_imap(&key, &credential))
        .await
        .map_err(|e| RpcError::new(error_code::INTERNAL_ERROR, e.to_string()))?
        .map_err(|e| RpcError::new(error_code::SECRETS, e))?;

    // 5: the file, then applied.
    let new = NewAccount {
        key: p.key.clone(),
        email: p.email.clone(),
        display_name: p.display_name.clone(),
        host: p.imap.host.clone(),
        port: p.imap.port,
        security: security_of(&p.imap)?,
        username: user,
        ca_file: p.imap.ca_file.clone(),
        read_only: p.read_only,
        smtp,
    };
    write_file(core, move |t| config_write::add_account(t, &new), &config_path).await?;
    Ok(AddAccountResult {
        account_id,
        created,
        roles: roles.into_iter().collect(),
        correspondents,
        archive_queued: p.archive_read_before.is_some() || p.archive_unread_before.is_some(),
    })
}

/// Writes the file with `change`, then applies it at once (the watch
/// would too, a moment later; reading the same text twice does nothing).
async fn write_file(
    core: &Arc<Core>,
    change: impl FnOnce(&str) -> Result<String, String> + Send + 'static,
    path: &std::path::Path,
) -> Result<(), RpcError> {
    let path = path.to_path_buf();
    let core2 = core.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        config_write::update(&path, change)?;
        crate::reload_config(&core2);
        Ok(())
    })
    .await
    .map_err(|e| RpcError::new(error_code::INTERNAL_ERROR, e.to_string()))?
    .map_err(|e| RpcError::new(error_code::INTERNAL_ERROR, e))
}

/// `set_folder_roles`: the roles of an existing account changed; the
/// folders named that the server lacks are created if asked.
pub async fn set_folder_roles(core: &Arc<Core>, p: SetFolderRolesParams, source: Source) -> Result<Vec<String>, RpcError> {
    human(source)?;
    let (secrets, account, id) = {
        let engine = core.engine.lock().unwrap();
        let account = engine.config().accounts.get(&p.account).cloned().ok_or_else(|| RpcError::new(error_code::NOT_FOUND, format!("no account {}", p.account)))?;
        let id = engine.account_id(&p.account).ok_or_else(|| RpcError::new(error_code::NOT_FOUND, format!("no account {}", p.account)))?;
        (engine.secrets(), account, id)
    };
    let key = p.account.clone();
    let acc = account.clone();
    let password = tokio::task::spawn_blocking(move || nm_engine::secrets::imap_password(secrets.as_ref(), &key, &acc))
        .await
        .map_err(|e| RpcError::new(error_code::INTERNAL_ERROR, e.to_string()))?
        .map_err(|r| RpcError::new(error_code::ACCOUNT_NOT_READY, r.message(&p.account, &account)))?;
    let spec = nm_engine::probe::imap_spec(&account);
    let mut s = setup::open(&spec, &account.imap.username, password.expose()).await.map_err(server)?;
    let entries = s.list().await.map_err(server)?;
    let existing: BTreeSet<String> = entries.iter().map(|e| e.name.name.clone()).collect();
    let paths: Vec<String> = p.roles.values().cloned().collect();
    let missing: Vec<&String> = paths.iter().filter(|x| !existing.contains(*x)).collect();
    let created = if p.create_missing {
        setup::create_folders(&mut s, &paths).await.map_err(server)?
    } else if let Some(m) = missing.first() {
        return Err(invalid(format!("the server has no folder {m:?}: ask to create it")));
    } else {
        Vec::new()
    };
    s.logout().await;
    let delimiter = entries.iter().find_map(|e| e.delimiter).map(String::from);
    let assignments: Vec<RoleAssignment> =
        p.roles.iter().map(|(r, path)| RoleAssignment { path: path.clone(), delimiter: delimiter.clone(), role: Some(*r) }).collect();
    let core2 = core.clone();
    with_store(core, move |st| {
        let changes = st.set_folder_roles(id, &assignments, source)?;
        core2.events.lock().unwrap().emit(&changes);
        Ok(())
    })
    .await?;
    if let Some(syncer) = core.syncer.get() {
        syncer.sync_now(Some(&p.account));
    }
    Ok(created)
}

/// `rename_account`: the database, the Keychain, then the file: the file
/// read next finds the account under its new key, data and password
/// included (`configuration.md`, section 6).
pub async fn rename_account(core: &Arc<Core>, p: RenameAccountParams, source: Source) -> Result<(), RpcError> {
    human(source)?;
    config_write::check_key(&p.to).map_err(invalid)?;
    let (path, secrets) = {
        let engine = core.engine.lock().unwrap();
        if !engine.config().accounts.contains_key(&p.from) {
            return Err(RpcError::new(error_code::NOT_FOUND, format!("no account {} in accounts.toml", p.from)));
        }
        if engine.config().accounts.contains_key(&p.to) {
            return Err(invalid(format!("accounts.toml already has an account {}", p.to)));
        }
        (engine.accounts_path(), engine.secrets())
    };
    if let Some(syncer) = core.syncer.get() {
        syncer.stop_and_join(&p.from, Duration::from_secs(5));
    }
    let (from, to) = (p.from.clone(), p.to.clone());
    with_store(core, move |st| st.rename_account(&from, &to, source)).await?;
    let (from, to) = (p.from.clone(), p.to.clone());
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        if let Some(c) = secrets.imap(&from)? {
            secrets.set_imap(&to, &c)?;
            secrets.remove_imap(&from)?;
        }
        // The SMTP password saved apart follows the account too.
        if let Some(c) = secrets.smtp(&from)? {
            secrets.set_smtp(&to, &c)?;
            secrets.remove_smtp(&from)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| RpcError::new(error_code::INTERNAL_ERROR, e.to_string()))?
    .map_err(|e| RpcError::new(error_code::SECRETS, e))?;
    let (from, to) = (p.from.clone(), p.to.clone());
    write_file(core, move |t| config_write::rename_account(t, &from, &to), &path).await
}

/// `remove_account`: the worker stopped and waited for, the file, the
/// data, the password.
pub async fn remove_account(core: &Arc<Core>, p: RemoveAccountParams, source: Source) -> Result<(), RpcError> {
    human(source)?;
    let (path, secrets) = {
        let engine = core.engine.lock().unwrap();
        if !engine.config().accounts.contains_key(&p.account) {
            return Err(RpcError::new(error_code::NOT_FOUND, format!("no account {} in accounts.toml", p.account)));
        }
        (engine.accounts_path(), engine.secrets())
    };
    if let Some(syncer) = core.syncer.get() {
        syncer.stop_and_join(&p.account, Duration::from_secs(5));
    }
    let key = p.account.clone();
    write_file(core, move |t| config_write::remove_account(t, &key), &path).await?;
    let key = p.account.clone();
    let core2 = core.clone();
    with_store(core, move |st| {
        let changes = st.remove_account(&key, source)?;
        core2.events.lock().unwrap().emit(&changes);
        Ok(())
    })
    .await?;
    let key = p.account.clone();
    tokio::task::spawn_blocking(move || secrets.remove_imap(&key).and_then(|()| secrets.remove_smtp(&key)))
        .await
        .map_err(|e| RpcError::new(error_code::INTERNAL_ERROR, e.to_string()))?
        .map_err(|e| RpcError::new(error_code::SECRETS, e))
}
