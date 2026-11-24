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
//!
//! An account created read-only (only looked at) gets the steps it skipped
//! later, from `activate_account`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use nm_api::reason;
use nm_api::{
    ActivateAccountParams, ActivateAccountResult, PrepareActivationParams, AddAccountParams, AddAccountResult, ArchiveCount, CountArchiveParams, DiscoverAccountParams, DiscoveryReport,
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
        Err(RpcError::new(error_code::FORBIDDEN, reason::RESERVED_TO_PEOPLE))
    }
}

/// A failure of the IMAP server, its answer cited as it is.
fn server(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(error_code::SERVER, reason::IMAP_FAILED.with("server_text", e.to_string()))
}

/// An IMAP failure of the creation methods, whose reason tells a client
/// which field to correct (`error_code::SERVER`): the password
/// (`imap_auth_refused`), the server (`imap_unreachable`), or neither
/// (`imap_failed`).
fn imap_failure(e: nm_proto::imap::ImapError) -> RpcError {
    use nm_proto::imap::ImapError as E;
    let why = match &e {
        E::AuthFailed(_) => reason::IMAP_AUTH_REFUSED,
        E::Connect(_) | E::Tls(_) | E::Timeout(_) | E::Closed | E::Bye(_) | E::Io(_) => reason::IMAP_UNREACHABLE,
        _ => reason::IMAP_FAILED,
    };
    RpcError::new(error_code::SERVER, why.with("server_text", e.to_string()))
}

/// A mistake of the client's program (`invalid_params`), `e` its detail.
fn invalid(e: impl std::fmt::Display) -> RpcError {
    RpcError::invalid(e)
}

/// A value the user gave that cannot be used: its reason.
fn refused(why: impl Into<nm_api::Why>) -> RpcError {
    RpcError::new(error_code::INVALID_PARAMS, why)
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The account's own addresses, lowercase.
fn own(email: &str, identities: &[String]) -> Vec<String> {
    std::iter::once(email).chain(identities.iter().map(String::as_str)).map(|a| a.trim().to_lowercase()).collect()
}

/// With `oauth: {flow}`, the IMAP server the client names must be one the
/// token may go to: a host of the compiled provider table, in TLS
/// (`oauth2.md`, section 7, "Serveurs autorisés"). Checked before any
/// connection: without it, a client could make the core send the flow's
/// access token, valid one hour for the whole mailbox, to any server that
/// announces `AUTH=XOAUTH2`.
fn oauth_server_allowed(oauth: Option<&nm_api::OauthFlow>, imap: &nm_api::ImapParams) -> Result<(), RpcError> {
    if oauth.is_none() {
        return Ok(());
    }
    if nm_engine::oauth2::provider_for_imap(&imap.host).is_none()
        || matches!(setup::spec_of(imap).map_err(invalid)?.security, nm_proto::imap::Security::None)
    {
        return Err(refused(reason::OAUTH2_UNKNOWN_SERVER.with("host", imap.host.as_str())));
    }
    Ok(())
}

/// `test_account`: the server tried, nothing written anywhere.
pub async fn test_account(core: &Arc<Core>, p: TestAccountParams, source: Source) -> Result<TestAccountReport, RpcError> {
    human(source)?;
    let spec = setup::spec_of(&p.imap).map_err(invalid)?;
    let user = setup::username_of(&p.imap, &p.email);
    let import = core.engine.lock().unwrap().config().import.clone();
    oauth_server_allowed(p.oauth.as_ref(), &p.imap)?;
    // `oauth: {flow}` replaces the password: the tokens stay in the core,
    // attached to the flow (`specifications/oauth2.md`, section 8).
    let tokens = match &p.oauth {
        Some(o) => Some(core.oauth.take_flow(&o.flow, &p.email).map_err(|w| RpcError::new(error_code::SERVER, w))?),
        None => None,
    };
    let cred = match &tokens {
        Some(t) => nm_engine::probe::Cred::Token(&t.access_token),
        None => nm_engine::probe::Cred::Password(&p.password),
    };
    setup::test_account(&spec, &user, cred, &own(&p.email, &p.identities), &import, now()).await.map_err(imap_failure)
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
    if nm_engine::providers::split_address(&p.email).is_err() {
        return Err(refused(reason::ADDRESS_INVALID.with("address", p.email.as_str())));
    }
    nm_engine::discover::discover(&p.email, &existing, p.imap.as_ref(), core.oauth.client_id().is_some()).await.map_err(invalid)
}

/// `test_smtp`: the candidates tried in order, the first that works
/// chosen. The IMAP password goes only to a server of the IMAP server's
/// organization (`nm_engine::discover::test_smtp`).
pub async fn test_smtp(core: &Arc<Core>, p: TestSmtpParams, source: Source) -> Result<SmtpTestReport, RpcError> {
    human(source)?;
    for c in &p.candidates {
        setup::spec_of(&c.server).map_err(invalid)?;
    }
    if p.smtp_password.as_deref() == Some("") {
        return Err(refused(reason::SMTP_PASSWORD_EMPTY));
    }
    let token = match &p.oauth {
        Some(o) => Some(core.oauth.take_flow(&o.flow, &p.email).map_err(|w| RpcError::new(error_code::SERVER, w))?.access_token),
        None => None,
    };
    Ok(nm_engine::discover::test_smtp(&p.email, &p.imap, &p.password, p.smtp_password.as_deref(), token.as_ref(), &p.candidates).await)
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
pub async fn count_archive(core: &Arc<Core>, p: CountArchiveParams, source: Source) -> Result<ArchiveCount, RpcError> {
    human(source)?;
    let spec = setup::spec_of(&p.imap).map_err(invalid)?;
    let user = setup::username_of(&p.imap, &p.email);
    for d in [&p.read_before, &p.unread_before].into_iter().flatten() {
        setup::parse_day(d).ok_or_else(|| refused(reason::DATE_INVALID.with("date", d.as_str())))?;
    }
    oauth_server_allowed(p.oauth.as_ref(), &p.imap)?;
    // `oauth: {flow}` replaces the password, as in `test_account`.
    let tokens = match &p.oauth {
        Some(o) => Some(core.oauth.take_flow(&o.flow, &p.email).map_err(|w| RpcError::new(error_code::SERVER, w))?),
        None => None,
    };
    let cred = match &tokens {
        Some(t) => nm_engine::probe::Cred::Token(&t.access_token),
        None => nm_engine::probe::Cred::Password(&p.password),
    };
    let mut s = setup::open(&spec, &user, cred).await.map_err(imap_failure)?;
    s.forbid_writes();
    let n = setup::count_archive(&mut s, p.uidnext, p.read_before.as_deref(), p.unread_before.as_deref()).await.map_err(imap_failure)?;
    s.logout().await;
    Ok(n)
}

/// The folder of each workflow role (`setup::plan_roles`), from the
/// server's folders.
fn roles_for(
    chosen: &BTreeMap<FolderRole, String>,
    entries: &[nm_proto::imap::ListEntry],
    namespace: Option<&nm_proto::imap::Namespace>,
) -> BTreeMap<FolderRole, String> {
    let proposed = nm_engine::folders::propose_roles(entries, namespace);
    let pairs: Vec<(&str, Option<&str>)> =
        entries.iter().zip(&proposed).map(|(e, r)| (e.name.name.as_str(), r.as_ref().map(|(r, _)| *r))).collect();
    setup::plan_roles(chosen, &pairs, namespace.map(|n| n.prefix.as_str()).unwrap_or(""))
}

/// `add_account`.
pub async fn add_account(core: &Arc<Core>, p: AddAccountParams, source: Source) -> Result<AddAccountResult, RpcError> {
    human(source)?;
    config_write::check_key(&p.key).map_err(invalid)?;
    // With `oauth: {flow}`, no password is asked: the flow's tokens are
    // stored in the Keychain instead (`specifications/oauth2.md`, section 8).
    if p.oauth.is_none() && p.password.is_empty() {
        return Err(refused(reason::PASSWORD_EMPTY));
    }
    if p.oauth.is_some() && p.smtp_password.is_some() {
        return Err(refused(reason::INVALID_PARAMS.with("detail", "an OAuth2 account has no SMTP password: the same token is used")));
    }
    oauth_server_allowed(p.oauth.as_ref(), &p.imap)?;
    // The flow must have succeeded, for this address.
    let tokens = match &p.oauth {
        Some(o) => Some(core.oauth.take_flow(&o.flow, &p.email).map_err(|w| RpcError::new(error_code::SERVER, w))?),
        None => None,
    };
    let (config_path, secrets, import) = {
        let engine = core.engine.lock().unwrap();
        if engine.config().accounts.contains_key(&p.key) {
            return Err(refused(reason::ACCOUNT_EXISTS.with("account", p.key.as_str())));
        }
        (engine.accounts_path(), engine.secrets(), engine.config().import.clone())
    };
    for d in [&p.archive_read_before, &p.archive_unread_before].into_iter().flatten() {
        setup::parse_day(d).ok_or_else(|| refused(reason::DATE_INVALID.with("date", d.as_str())))?;
    }
    if p.read_only && (p.archive_read_before.is_some() || p.archive_unread_before.is_some()) {
        return Err(refused(reason::READ_ONLY_ARCHIVES_NOTHING));
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
    let smtp_password = match (&p.smtp_password, &smtp) {
        (None, _) => None,
        (Some(pw), _) if pw.is_empty() => return Err(refused(reason::SMTP_PASSWORD_EMPTY)),
        (Some(_), None) => return Err(refused(reason::SMTP_PASSWORD_WITHOUT_SERVER)),
        (Some(pw), Some(s)) => Some(Credential {
            binding: Binding { host: s.host.clone(), username: s.username.clone() },
            secret: nm_engine::config::SecretString::new(pw.clone()),
        }),
    };

    // 1 and 2: the server.
    let cred = match &tokens {
        Some(t) => nm_engine::probe::Cred::Token(&t.access_token),
        None => nm_engine::probe::Cred::Password(&p.password),
    };
    let mut s = setup::open(&spec, &user, cred).await.map_err(imap_failure)?;
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

    // 4: the secret, for this server: the password, or the OAuth2 entry
    // (the flow's refresh token) of the flow, stored in the same Keychain
    // item, bound to this server and user name. Saved before the file, so
    // that the worker the file starts finds it.
    let key = p.key.clone();
    let secrets2 = secrets.clone();
    let oauth2 = core.oauth.clone();
    let (flow, email2, host2, user2, password2) =
        (p.oauth.as_ref().map(|o| o.flow.clone()), p.email.clone(), p.imap.host.clone(), user.clone(), p.password.clone());
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        match &flow {
            Some(flow) => {
                oauth2.store_flow(&key, &email2, &host2, &user2, secrets2.as_ref(), flow).map_err(|w| w.message())?;
            }
            None => {
                let credential = Credential {
                    binding: Binding { host: host2, username: user2 },
                    secret: nm_engine::config::SecretString::new(password2),
                };
                secrets2.set_imap(&key, &credential)?;
            }
        }
        match &smtp_password {
            Some(c) => secrets2.set_smtp(&key, c),
            None => Ok(()),
        }
    })
    .await
    .map_err(|e| RpcError::internal(e))?
    .map_err(|e| RpcError::new(error_code::SECRETS, reason::KEYCHAIN.with("system_text", e)))?;

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
        auth: p.oauth.as_ref().map(|_| "oauth2".to_string()),
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
    .map_err(|e| RpcError::internal(e))?
    .map_err(|e| RpcError::internal(e))
}

/// `set_folder_roles`: the roles of an existing account changed; the
/// folders named that the server lacks are created if asked.
pub async fn set_folder_roles(core: &Arc<Core>, p: SetFolderRolesParams, source: Source) -> Result<Vec<String>, RpcError> {
    human(source)?;
    let (secrets, account, id) = {
        let engine = core.engine.lock().unwrap();
        let account = engine.config().accounts.get(&p.account).cloned().ok_or_else(|| RpcError::new(error_code::NOT_FOUND, reason::ACCOUNT_UNKNOWN.with("account", p.account.as_str())))?;
        let id = engine.account_id(&p.account).ok_or_else(|| RpcError::new(error_code::NOT_FOUND, reason::ACCOUNT_UNKNOWN.with("account", p.account.as_str())))?;
        (engine.secrets(), account, id)
    };
    let key = p.account.clone();
    let acc = account.clone();
    let password = tokio::task::spawn_blocking(move || nm_engine::secrets::imap_password(secrets.as_ref(), &key, &acc))
        .await
        .map_err(|e| RpcError::internal(e))?
        .map_err(|r| RpcError::new(error_code::ACCOUNT_NOT_READY, r.why(&p.account, &account)))?;
    let spec = nm_engine::probe::imap_spec(&account);
    let mut s = setup::open(&spec, &account.imap.username, nm_engine::probe::Cred::Password(password.expose())).await.map_err(server)?;
    let entries = s.list().await.map_err(server)?;
    let existing: BTreeSet<String> = entries.iter().map(|e| e.name.name.clone()).collect();
    let paths: Vec<String> = p.roles.values().cloned().collect();
    let missing: Vec<&String> = paths.iter().filter(|x| !existing.contains(*x)).collect();
    let created = if p.create_missing {
        setup::create_folders(&mut s, &paths).await.map_err(server)?
    } else if let Some(m) = missing.first() {
        return Err(refused(reason::SERVER_FOLDER_MISSING.with("folder", m.as_str())));
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

/// A read-only account of the file, its id and its password from the
/// Keychain: what `prepare_activation` and `activate_account` start from.
async fn read_only_account(
    core: &Arc<Core>,
    key: &str,
) -> Result<(nm_engine::config::AccountConfig, i64, nm_engine::config::SecretString), RpcError> {
    let (secrets, account, id) = {
        let engine = core.engine.lock().unwrap();
        let missing = || RpcError::new(error_code::NOT_FOUND, reason::ACCOUNT_UNKNOWN.with("account", key));
        let account = engine.config().accounts.get(key).cloned().ok_or_else(missing)?;
        let id = engine.account_id(key).ok_or_else(missing)?;
        (engine.secrets(), account, id)
    };
    if !account.read_only {
        return Err(refused(reason::ACCOUNT_SET_UP.with("account", key)));
    }
    let (k, acc) = (key.to_string(), account.clone());
    let password = tokio::task::spawn_blocking(move || nm_engine::secrets::imap_password(secrets.as_ref(), &k, &acc))
        .await
        .map_err(|e| RpcError::internal(e))?
        .map_err(|r| RpcError::new(error_code::ACCOUNT_NOT_READY, r.why(key, &account)))?;
    Ok((account, id, password))
}

/// The account's own addresses: its address and its identities.
fn own_of(account: &nm_engine::config::AccountConfig) -> Vec<String> {
    let identities: Vec<String> = account.identities.iter().map(|i| i.email.clone()).collect();
    own(&account.email, &identities)
}

/// `prepare_activation`: the report of `test_account` for a read-only
/// account, its password taken from the Keychain. Nothing is written: the
/// connection is read-only too.
pub async fn prepare_activation(core: &Arc<Core>, p: PrepareActivationParams, source: Source) -> Result<TestAccountReport, RpcError> {
    human(source)?;
    let (account, _, password) = read_only_account(core, &p.account).await?;
    let spec = nm_engine::probe::imap_spec(&account);
    setup::test_account(&spec, &account.imap.username, nm_engine::probe::Cred::Password(password.expose()), &own_of(&account), &account.import, now())
        .await
        .map_err(imap_failure)
}

/// `activate_account`: a read-only account made writable, with the steps
/// of `add_account` that read-only skipped (`docs/accounts.md`, "Looking
/// first"). In this order, for the same reason: the worker that the file
/// restarts writable finds everything ready.
/// 1. log in; create the folders the roles need; read the boundary again
///    and the recipients of Sent;
/// 2. database: the roles, the boundary set again (the mails that came
///    while the account was looked at are not screened: they are already
///    there, `import-initial.md`, section 1), the correspondents, the
///    archiving queued;
/// 3. `read_only` removed from `accounts.toml`, applied at once: the
///    read-only worker stops, a writable one starts, sends the archiving,
///    then synchronizes.
///
/// A failure before step 3 leaves the account read-only: the folders
/// created stay (asked again, they are found), and the call can be made
/// again.
pub async fn activate_account(core: &Arc<Core>, p: ActivateAccountParams, source: Source) -> Result<ActivateAccountResult, RpcError> {
    human(source)?;
    for d in [&p.archive_read_before, &p.archive_unread_before].into_iter().flatten() {
        setup::parse_day(d).ok_or_else(|| refused(reason::DATE_INVALID.with("date", d.as_str())))?;
    }
    let (account, id, password) = read_only_account(core, &p.account).await?;
    let config_path = core.engine.lock().unwrap().accounts_path();
    let own = own_of(&account);

    // 1: the server.
    let spec = nm_engine::probe::imap_spec(&account);
    let mut s = setup::open(&spec, &account.imap.username, nm_engine::probe::Cred::Password(password.expose())).await.map_err(imap_failure)?;
    let namespace = s.namespace().await.ok().and_then(|n| n.personal.into_iter().next());
    let entries = s.list().await.map_err(server)?;
    let roles = roles_for(&p.roles, &entries, namespace.as_ref());
    let paths: Vec<String> = roles.values().cloned().collect();
    let created = setup::create_folders(&mut s, &paths).await.map_err(server)?;
    let boundary = match p.uidnext {
        Some(u) => u,
        None => s.status(&nm_proto::imap::MailboxName::inbox()).await.map_err(server)?.uid_next.unwrap_or(1),
    };
    let recipients = match (p.correspondents, roles.get(&FolderRole::Sent)) {
        (true, Some(sent)) => setup::sent_recipients(&mut s, sent, &own).await.map_err(server)?,
        _ => Vec::new(),
    };
    s.logout().await;

    // 2: the database.
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
    let archive = (p.archive_read_before.is_some() || p.archive_unread_before.is_some()).then(|| ArchiveCriteria {
        from_path: "INBOX".into(),
        to_path: roles.get(&FolderRole::Archive).cloned().unwrap_or_default(),
        boundary,
        read_before: p.archive_read_before.clone(),
        unread_before: p.archive_unread_before.clone(),
        batch_size: account.import.batch_size as u32,
    });
    let core2 = core.clone();
    let correspondents = with_store(core, move |st| {
        let changes = st.set_folder_roles(id, &assignments, source)?;
        core2.events.lock().unwrap().emit(&changes);
        let inbox: i64 = st.conn().query_row("SELECT id FROM folder WHERE account_id = ?1 AND role = 'inbox'", [id], |r| r.get(0))?;
        st.reset_screening_start(inbox, boundary)?;
        let n = st.add_correspondents(&recipients, &own)?;
        if let Some(c) = &archive {
            st.queue_initial_archive(id, c, source)?;
        }
        Ok(n)
    })
    .await?;

    // 3: the file, then applied.
    let key = p.account.clone();
    write_file(core, move |t| config_write::clear_read_only(t, &key), &config_path).await?;
    Ok(ActivateAccountResult {
        created,
        roles: roles.into_iter().collect(),
        correspondents,
        archive_queued: p.archive_read_before.is_some() || p.archive_unread_before.is_some(),
    })
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
            return Err(RpcError::new(error_code::NOT_FOUND, reason::ACCOUNT_UNKNOWN.with("account", p.from.as_str())));
        }
        if engine.config().accounts.contains_key(&p.to) {
            return Err(refused(reason::ACCOUNT_EXISTS.with("account", p.to.as_str())));
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
    .map_err(|e| RpcError::internal(e))?
    .map_err(|e| RpcError::new(error_code::SECRETS, reason::KEYCHAIN.with("system_text", e)))?;
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
            return Err(RpcError::new(error_code::NOT_FOUND, reason::ACCOUNT_UNKNOWN.with("account", p.account.as_str())));
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
        .map_err(|e| RpcError::internal(e))?
        .map_err(|e| RpcError::new(error_code::SECRETS, reason::KEYCHAIN.with("system_text", e)))
}
