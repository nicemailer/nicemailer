//! OAuth2 sign-in to Microsoft 365 and Outlook.com accounts
//! (`specifications/oauth2.md`).
//!
//! The core does everything but open the browser: it listens on the
//! loopback for the redirection (RFC 8252), exchanges the code with PKCE
//! (RFC 7636), checks the `id_token`, keeps the tokens and renews them.
//! Clients only open the URL the core gives them and wait for the
//! `OAuthFinished` event. No client secret: a desktop app cannot keep
//! one, so the app is a *public* client and PKCE protects the exchange.
//!
//! The access token never touches the disk: it lives here, in memory,
//! with its expiry. Only the refresh token is stored, in the Keychain,
//! in place of the IMAP password (`secrets.rs`).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nm_api::{Event, Why, reason as r};
use nm_core::secret::SecretString;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::config::{AccountConfig, AuthMethod};
use crate::secrets::{Binding, OauthCredential, Secrets};

/// The application id compiled in the core (`oauth2.md`, section 4):
/// `NICEMAILER_MICROSOFT_CLIENT_ID` at build time, checked by `build.rs`
/// (a malformed value fails the build), exposed as `NICEMAILER_COMPILED_CLIENT_ID`.
/// A development build without the variable embeds the zero GUID: then no
/// id is available, OAuth2-only servers are not usable, and `oauth_begin`
/// fails cleanly. The id is not a secret — a public client sends it in
/// every authorization URL — so compiling it in is safe; no client secret
/// is compiled in, the flow relies on PKCE and the loopback redirect.
pub const COMPILED_CLIENT_ID: &str = env!("NICEMAILER_COMPILED_CLIENT_ID");

pub const AUTHORIZE_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
pub const TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
/// The delegated permissions of the app (`oauth2.md`, section 4): the two
/// mail ones belong to the Office 365 Exchange Online resource, so one
/// access token serves IMAP and SMTP.
///
/// They are named with their resource (`https://outlook.office.com/`), as
/// Microsoft documents for IMAP and SMTP. A bare `IMAP.AccessAsUser.All`
/// is taken by the v2 endpoint as a permission of Microsoft Graph, which
/// has one of the same name: the sign-in then succeeds, but the access
/// token is for Graph, and `outlook.office365.com` refuses it
/// (`AUTHENTICATE failed`).
pub const SCOPES: &str =
    "https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send offline_access openid email profile";

/// A flow expires after 10 minutes without redirection (`oauth2.md`,
/// section 5). Successful flows whose tokens nobody used are forgotten
/// after the same delay.
const FLOW_TIMEOUT: Duration = Duration::from_secs(600);
/// The listener reads at most 8 KiB of headers, in 5 seconds at most.
const MAX_HEADERS: usize = 8192;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Renew the access token when it has less than 5 minutes left.
const RENEW_MARGIN: Duration = Duration::from_secs(300);

/// A provider that accepts OAuth2, with the servers a token may be sent
/// to. Compiled in the core, never read from `accounts.toml`: reading the
/// list there would let anyone who can edit the file receive a token
/// valid one hour for the whole mailbox (`oauth2.md`, section 7).
pub struct OauthProvider {
    pub name: &'static str,
    pub imap: &'static [&'static str],
    pub smtp: &'static [&'static str],
}

pub const PROVIDERS: &[OauthProvider] = &[OauthProvider {
    name: "microsoft",
    imap: &["outlook.office365.com"],
    smtp: &["smtp.office365.com", "smtp-mail.outlook.com"],
}];

/// The OAuth2 provider of an IMAP host, if the host is in the table.
pub fn provider_for_imap(host: &str) -> Option<&'static OauthProvider> {
    let host = host.to_ascii_lowercase();
    PROVIDERS.iter().find(|p| p.imap.iter().any(|h| *h == host))
}

/// The OAuth2 provider of an SMTP host, if the host is in the table.
pub fn provider_for_smtp(host: &str) -> Option<&'static OauthProvider> {
    let host = host.to_ascii_lowercase();
    PROVIDERS.iter().find(|p| p.smtp.iter().any(|h| *h == host))
}

/// Whether a token of the account's provider may be sent to `smtp_host`:
/// the SMTP host must be in the same provider's table entry (`oauth2.md`,
/// section 7). Without this check, a file changed to point `[smtp]`
/// elsewhere would receive the token.
pub fn smtp_host_allowed(account_imap_host: &str, smtp_host: &str) -> bool {
    match (provider_for_imap(account_imap_host), provider_for_smtp(smtp_host)) {
        (Some(a), Some(b)) => std::ptr::eq(a, b),
        _ => false,
    }
}

/// Whether the provider of an address (found by domain in the database)
/// accepts OAuth2. An unknown domain is accepted: its MX decides later,
/// at discovery (`oauth2.md`, section 7).
pub fn provider_accepts_email(email: &str) -> bool {
    let Some((_, domain)) = email.split_once('@') else { return false };
    let Ok(db) = crate::providers::Providers::get() else { return true };
    match db.by_domain(&domain.to_lowercase()) {
        None => true,
        Some(p) => p.imap.iter().any(|s| provider_for_imap(&s.host).is_some()),
    }
}

/// The client id in effect: the `oauth.microsoft_client_id` setting when
/// it is set, else the compiled one. `None` when neither is usable (the
/// app is not registered yet).
pub fn client_id_of(oauth: &crate::config::OauthSettings) -> Option<String> {
    match &oauth.microsoft_client_id {
        Some(id) => Some(id.clone()),
        // A zero GUID means "not registered yet": no id is available.
        None if crate::config::is_client_id(COMPILED_CLIENT_ID) => Some(COMPILED_CLIENT_ID.to_string()),
        None => None,
    }
}

// ---------------------------------------------------------------------------
// base64url and percent encoding (PKCE, query strings, form bodies)
// ---------------------------------------------------------------------------

/// base64url without padding (RFC 7636 uses it for the verifier).
fn b64url(bytes: &[u8]) -> String {
    const URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(URL[((n >> 18) & 63) as usize] as char);
        out.push(URL[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(URL[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(URL[(n & 63) as usize] as char);
        }
    }
    out
}

/// Decodes base64url (padding tolerated): the payload of an `id_token`.
fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut nbits = 0;
    for &b in s.as_bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
        }
    }
    Some(out)
}

/// Percent-encodes one value of a query string or a form body.
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `a=1&b=2`, a form body or a query string.
fn form_encode(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", pct(k), pct(v))).collect::<Vec<_>>().join("&")
}

/// Decodes one query-string value (`+` is a space). An invalid escape is
/// kept as it is: a malformed redirection fails the `state` check anyway.
fn pct_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                match std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parses a query string into a map.
fn query_params(query: &str) -> BTreeMap<String, String> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (pct_decode(k), pct_decode(v)),
            None => (pct_decode(p), String::new()),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

/// The tokens of a finished flow, or of an account already stored. The
/// access token lives only here, in memory.
#[derive(Debug, Clone)]
pub struct FlowTokens {
    pub access_token: SecretString,
    pub access_expires: Instant,
    pub refresh_token: SecretString,
    pub client_id: String,
    pub tenant: Option<String>,
    pub user_id: Option<String>,
    pub email: String,
}

/// What failed to give a token.
#[derive(Debug, Clone)]
pub enum TokenError {
    /// Network error, `5xx`, `temporarily_unavailable`: retry like any
    /// network error; the account is not put in error.
    Temporary(String),
    /// The refresh token was refused (`invalid_grant`, `interaction_required`),
    /// no entry is stored, or the application id changed: sign in again.
    Reauth(Why),
    /// Another explicit refusal of the provider (`oauth2.md`, section 6).
    Refused(Why),
}

impl TokenError {
    pub fn why(&self) -> Why {
        match self {
            TokenError::Temporary(m) => r::IMAP_UNREACHABLE.with("server_text", m.as_str()),
            TokenError::Reauth(w) | TokenError::Refused(w) => w.clone(),
        }
    }

    pub fn is_temporary(&self) -> bool {
        matches!(self, TokenError::Temporary(_))
    }
}

/// A success answer of the token endpoint.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

/// An error answer of the token endpoint.
#[derive(Deserialize)]
struct TokenErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// The claims of an `id_token` the core reads: `nonce` to check, the
/// address to compare, `tid`/`oid` to keep (`oauth2.md`, section 5).
#[derive(Deserialize)]
struct IdTokenClaims {
    nonce: Option<String>,
    email: Option<String>,
    preferred_username: Option<String>,
    tid: Option<String>,
    oid: Option<String>,
    sub: Option<String>,
}

/// Maps an error of the token endpoint to a reason (`oauth2.md`, section
/// 6). The `AADSTS…` code and Microsoft's text go into the values and the
/// journal, in English: that is what an administrator will ask for.
fn token_error_why(account: Option<&str>, error: &str, description: &str) -> TokenError {
    let system_text = format!("{error}: {}", description.trim());
    eprintln!("oauth2: token endpoint refused: {system_text}");
    let reauth = |w: Why| TokenError::Reauth(match account {
        Some(key) => w.with("account", key),
        None => w,
    });
    if error == "invalid_grant" || error == "interaction_required" {
        return reauth(r::OAUTH_REAUTH_NEEDED.into());
    }
    if description.contains("AADSTS65001") || description.contains("AADSTS90094") {
        return TokenError::Refused(r::OAUTH_ADMIN_CONSENT_NEEDED.with("system_text", system_text));
    }
    if description.contains("AADSTS53003") {
        return TokenError::Refused(r::OAUTH_BLOCKED_BY_POLICY.with("system_text", system_text));
    }
    if error == "invalid_client" || error == "unauthorized_client" {
        return TokenError::Refused(r::OAUTH_CLIENT_REJECTED.with("system_text", system_text));
    }
    if error == "temporarily_unavailable" {
        return TokenError::Temporary(system_text);
    }
    // An unknown 4xx is a configuration problem: signing in again is the
    // remedy. A 5xx or a network failure is temporary.
    if error.starts_with('4') || !error.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        reauth(r::OAUTH_REAUTH_NEEDED.into())
    } else {
        TokenError::Temporary(system_text)
    }
}

// ---------------------------------------------------------------------------
// The flow manager
// ---------------------------------------------------------------------------

struct Flow {
    email: String,
    account: Option<String>,
    state: String,
    nonce: String,
    verifier: String,
    port: u16,
    created: Instant,
    cancel: tokio::sync::watch::Sender<bool>,
    /// Set when the flow ended successfully; the tokens stay attached to
    /// the flow until `add_account` stores them (`oauth2.md`, section 5).
    tokens: Option<FlowTokens>,
}

#[derive(Default)]
struct Inner {
    flows: BTreeMap<String, Flow>,
    /// Access tokens in memory, by account key. Never written to disk.
    tokens: BTreeMap<String, FlowTokens>,
    /// One renewal at a time per account: the connections of an account
    /// (folders, bodies, IDLE, SMTP) may need a token at the same instant.
    refresh_locks: BTreeMap<String, Arc<tokio::sync::Mutex<()>>>,
    /// Rotated refresh tokens the Keychain refused to store (locked), by
    /// account key. The provider may have invalidated the old one, which is
    /// still in the Keychain: the next renewal presents this one instead,
    /// and writes it again. Forgotten once written, or when the account is
    /// signed in again.
    unsaved_refresh: BTreeMap<String, SecretString>,
}

/// What `oauth_begin` started.
pub struct OauthBegin {
    pub flow: String,
    pub url: String,
    pub provider: &'static str,
}

/// The OAuth2 state of the core: the running flows, the in-memory access
/// tokens, and the renewals. Shared by the daemon and the sync workers.
pub struct Oauth {
    inner: Mutex<Inner>,
    authorize_url: Mutex<String>,
    token_url: Mutex<String>,
    /// false only in tests, whose fake token endpoint is plain HTTP.
    https_only: Mutex<bool>,
    client_id: Mutex<Option<String>>,
    sink: Mutex<Option<Arc<dyn Fn(Event) + Send + Sync>>>,
}

impl Default for Oauth {
    fn default() -> Self {
        Self::new()
    }
}

impl Oauth {
    pub fn new() -> Oauth {
        Oauth {
            inner: Mutex::new(Inner::default()),
            authorize_url: Mutex::new(AUTHORIZE_URL.to_string()),
            token_url: Mutex::new(TOKEN_URL.to_string()),
            https_only: Mutex::new(true),
            client_id: Mutex::new(client_id_of(&crate::config::OauthSettings::default())),
            sink: Mutex::new(None),
        }
    }

    /// The endpoints, for tests (a fake authorization/token server).
    pub fn set_endpoints(&self, authorize_url: &str, token_url: &str, https_only: bool) {
        *self.authorize_url.lock().unwrap() = authorize_url.to_string();
        *self.token_url.lock().unwrap() = token_url.to_string();
        *self.https_only.lock().unwrap() = https_only;
    }

    /// The application id in effect, from the settings.
    pub fn set_client_id(&self, id: Option<String>) {
        *self.client_id.lock().unwrap() = id;
    }

    pub fn client_id(&self) -> Option<String> {
        self.client_id.lock().unwrap().clone()
    }

    /// Where finished flows announce themselves (`OAuthFinished`).
    pub fn set_sink(&self, sink: Arc<dyn Fn(Event) + Send + Sync>) {
        *self.sink.lock().unwrap() = Some(sink);
    }

    fn emit(&self, event: Event) {
        if let Some(sink) = self.sink.lock().unwrap().clone() {
            sink(event);
        }
    }

    /// `oauth_begin {email}` or `{account}` (`oauth2.md`, section 5): opens
    /// the loopback listeners, builds the authorization URL, spawns the
    /// flow, and returns what the client needs to open the browser. A new
    /// flow for the same address cancels the previous one.
    pub async fn begin(self: &Arc<Self>, email: &str, account: Option<String>) -> Result<OauthBegin, Why> {
        if account.is_none() && !provider_accepts_email(email) {
            return Err(r::OAUTH2_UNSUPPORTED_PROVIDER.with("email", email));
        }
        let Some(client_id) = self.client_id() else {
            return Err(r::OAUTH_CLIENT_REJECTED.with("system_text", "no application id (the app is not registered yet)"));
        };
        // Listen on the loopback, the same port for IPv4 and IPv6:
        // `localhost` may resolve to either, according to the browser.
        let l4 = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| r::IMAP_UNREACHABLE.with("server_text", format!("loopback listen: {e}")))?;
        let port = l4.local_addr().map_err(|e| r::IMAP_UNREACHABLE.with("server_text", e.to_string()))?.port();
        let l6 = match TcpListener::bind(("::1", port)).await {
            Ok(l) => Some(l),
            Err(_) => None, // no IPv6 on this Mac: the redirect uses 127.0.0.1
        };
        let redirect_uri = format!("http://127.0.0.1:{port}");
        let id = format!("{:016x}{:016x}", rand::random::<u64>(), rand::random::<u64>());
        let verifier = b64url(&rand::random::<[u8; 32]>());
        let state = b64url(&rand::random::<[u8; 16]>());
        let nonce = b64url(&rand::random::<[u8; 16]>());
        let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
        let url = format!(
            "{}?{}",
            self.authorize_url.lock().unwrap(),
            form_encode(&[
                ("response_type", "code"),
                ("client_id", &client_id),
                ("redirect_uri", &redirect_uri),
                ("scope", SCOPES),
                ("state", &state),
                ("nonce", &nonce),
                ("code_challenge", &challenge),
                ("code_challenge_method", "S256"),
                ("login_hint", email),
                ("prompt", "select_account"),
            ])
        );
        let (cancel, cancel_rx) = tokio::sync::watch::channel(false);
        let mut cancelled_events = Vec::new();
        {
            let mut inner = self.inner.lock().unwrap();
            // Forget flows older than the timeout; their driver already
            // ended them. A new flow for the same address cancels the
            // previous one.
            let stale: Vec<String> = inner.flows.iter().filter(|(_, f)| f.created.elapsed() > FLOW_TIMEOUT).map(|(k, _)| k.clone()).collect();
            for k in stale {
                if let Some(f) = inner.flows.remove(&k) {
                    let _ = f.cancel.send(true);
                }
            }
            let cancelled: Vec<String> = inner
                .flows
                .iter()
                // Two flows for new accounts both have no key: only the
                // address tells them apart. A flow for one address must not
                // cancel another one's, finished and holding its tokens
                // until `add_account`.
                .filter(|(_, f)| f.email.eq_ignore_ascii_case(email) || (account.is_some() && f.account.as_deref() == account.as_deref()))
                .map(|(k, _)| k.clone())
                .collect();
            for k in cancelled {
                if let Some(f) = inner.flows.remove(&k) {
                    let _ = f.cancel.send(true);
                    cancelled_events.push(Event::OAuthFinished {
                        flow: k,
                        ok: false,
                        account: f.account,
                        email: f.email,
                        why: Some(Why::from(r::OAUTH_CANCELLED).data()),
                    });
                }
            }
            inner.flows.insert(
                id.clone(),
                Flow { email: email.to_string(), account, state, nonce, verifier, port, created: Instant::now(), cancel, tokens: None },
            );
        }
        for ev in cancelled_events {
            self.emit(ev);
        }
        let oauth = self.clone();
        let driver_flow = id.clone();
        tokio::spawn(async move {
            oauth.drive(driver_flow, l4, l6, cancel_rx).await;
        });
        Ok(OauthBegin { flow: id, url, provider: "microsoft" })
    }

    /// `oauth_cancel {flow}`: closes the listener, forgets the flow, and
    /// announces `oauth_cancelled` (unless the flow already ended).
    pub fn cancel(&self, flow: &str) {
        let f = self.inner.lock().unwrap().flows.remove(flow);
        if let Some(f) = f {
            let _ = f.cancel.send(true);
            if f.tokens.is_none() {
                self.emit(Event::OAuthFinished {
                    flow: flow.to_string(),
                    ok: false,
                    account: f.account,
                    email: f.email,
                    why: Some(Why::from(r::OAUTH_CANCELLED).data()),
                });
            }
        }
    }

    /// The tokens of a finished flow, for `test_account`/`test_smtp`/
    /// `add_account` (`oauth: {flow}` instead of a password). The flow must
    /// have succeeded and be for the same address.
    pub fn take_flow(&self, flow: &str, email: &str) -> Result<FlowTokens, Why> {
        let inner = self.inner.lock().unwrap();
        let f = inner.flows.get(flow).ok_or_else(|| r::INVALID_PARAMS.with("flow", flow))?;
        let tokens = f.tokens.clone().ok_or_else(|| Why::from(r::OAUTH_DENIED))?;
        if !tokens.email.eq_ignore_ascii_case(email) {
            return Err(r::OAUTH_WRONG_ACCOUNT.with("authorized", tokens.email.as_str()).with("email", email));
        }
        Ok(tokens)
    }

    /// `add_account` with `oauth`: stores the flow's tokens in the
    /// Keychain (in the `imap:<key>` item, in place of a password, bound to
    /// the server and user name the file names) and keeps the access token
    /// in memory. The flow is forgotten.
    pub fn store_flow(&self, key: &str, email: &str, imap_host: &str, username: &str, secrets: &dyn Secrets, flow: &str) -> Result<(), Why> {
        let tokens = self.take_flow(flow, email)?;
        let cred = OauthCredential {
            binding: Binding { host: imap_host.to_string(), username: username.to_string() },
            refresh_token: tokens.refresh_token.clone(),
            provider: "microsoft".to_string(),
            client_id: tokens.client_id.clone(),
            tenant: tokens.tenant.clone(),
            user_id: tokens.user_id.clone(),
            email: tokens.email.clone(),
        };
        secrets.set_oauth(key, &cred).map_err(|e| r::KEYCHAIN.with("system_text", e))?;
        let mut inner = self.inner.lock().unwrap();
        inner.tokens.insert(key.to_string(), tokens);
        // The new sign-in's refresh token replaces any rotated one still
        // waiting to be written.
        inner.unsaved_refresh.remove(key);
        inner.flows.remove(flow);
        Ok(())
    }

    /// An access token for the account, renewed when it has less than 5
    /// minutes left (`oauth2.md`, section 6). One renewal at a time per
    /// account: the other connections wait for its result.
    pub async fn access_token(&self, key: &str, account: &AccountConfig, secrets: &dyn Secrets) -> Result<SecretString, TokenError> {
        self.token(key, account, secrets, false).await
    }

    /// The same, ignoring the cached token: after the server refused one
    /// the core believed valid, get a new one and retry once.
    pub async fn access_token_forced(&self, key: &str, account: &AccountConfig, secrets: &dyn Secrets) -> Result<SecretString, TokenError> {
        self.token(key, account, secrets, true).await
    }

    async fn token(&self, key: &str, account: &AccountConfig, secrets: &dyn Secrets, force: bool) -> Result<SecretString, TokenError> {
        debug_assert!(account.auth == AuthMethod::Oauth2);
        if provider_for_imap(&account.imap.host).is_none() {
            return Err(TokenError::Refused(r::OAUTH2_UNKNOWN_SERVER.with("host", account.imap.host.as_str())));
        }
        if !force
            && let Some(t) = self.inner.lock().unwrap().tokens.get(key)
            && t.access_expires > Instant::now() + RENEW_MARGIN
        {
            return Ok(t.access_token.clone());
        }
        let lock = {
            let mut inner = self.inner.lock().unwrap();
            inner.refresh_locks.entry(key.to_string()).or_default().clone()
        };
        let _guard = lock.lock().await;
        // Another connection may have renewed while we waited.
        if !force
            && let Some(t) = self.inner.lock().unwrap().tokens.get(key)
            && t.access_expires > Instant::now() + RENEW_MARGIN
        {
            return Ok(t.access_token.clone());
        }
        let cred = match crate::secrets::oauth_credential(secrets, key, account) {
            Ok(Some(c)) => c,
            Ok(None) => return Err(TokenError::Reauth(r::OAUTH_REAUTH_NEEDED.with("account", key))),
            Err(e) => return Err(TokenError::Refused(e.why(key, account))),
        };
        // The tokens are bound to the id that got them: a changed id asks
        // for a new sign-in (`oauth2.md`, section 4).
        if self.client_id().as_deref() != Some(cred.client_id.as_str()) {
            return Err(TokenError::Reauth(r::OAUTH_REAUTH_NEEDED.with("account", key)));
        }
        // A rotated refresh token the Keychain refused last time is newer
        // than the one it holds: presented instead (the item read above
        // still vouches for the binding to the server).
        let mut cred = cred;
        let pending = self.inner.lock().unwrap().unsaved_refresh.get(key).cloned();
        if let Some(rt) = &pending {
            cred.refresh_token = rt.clone();
        }
        let resp = self.refresh(&cred, Some(key)).await?;
        let mut new_cred = cred.clone();
        if let Some(rt) = &resp.refresh_token {
            new_cred.refresh_token = SecretString::new(rt.clone());
        }
        // Rotation: the new refresh token is stored *before* the access
        // token is used. If the write fails (locked Keychain), the access
        // token is still used, the new refresh token is kept in memory,
        // presented at the next renewal and written again then.
        if resp.refresh_token.is_some() || pending.is_some() {
            let written = secrets.set_oauth(key, &new_cred);
            let mut inner = self.inner.lock().unwrap();
            match written {
                Ok(()) => {
                    inner.unsaved_refresh.remove(key);
                }
                Err(e) => {
                    eprintln!("oauth2 {key}: the Keychain refused the new refresh token, kept in memory: {e}");
                    inner.unsaved_refresh.insert(key.to_string(), new_cred.refresh_token.clone());
                }
            }
        }
        let tokens = FlowTokens {
            access_token: SecretString::new(resp.access_token),
            access_expires: Instant::now() + Duration::from_secs(resp.expires_in.unwrap_or(3600)),
            refresh_token: new_cred.refresh_token.clone(),
            client_id: cred.client_id.clone(),
            tenant: cred.tenant.clone(),
            user_id: cred.user_id.clone(),
            email: cred.email.clone(),
        };
        let access = tokens.access_token.clone();
        self.inner.lock().unwrap().tokens.insert(key.to_string(), tokens);
        Ok(access)
    }

    /// POSTs to the token endpoint (blocking HTTP, off the async runtime).
    async fn token_post(&self, form: &str) -> Result<TokenResponse, TokenError> {
        let url = self.token_url.lock().unwrap().clone();
        let https_only = *self.https_only.lock().unwrap();
        let form = form.to_string();
        tokio::task::spawn_blocking(move || -> Result<TokenResponse, TokenError> {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(30)))
                .https_only(https_only)
                .max_redirects(0)
                .user_agent("NiceMailer")
                .http_status_as_error(false)
                .build()
                .into();
            let response = agent
                .post(&url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .send(&form)
                .map_err(|e| TokenError::Temporary(format!("token endpoint: {e}")))?;
            let status = response.status().as_u16();
            let body = response.into_body().read_to_string().map_err(|e| TokenError::Temporary(format!("token endpoint: {e}")))?;
            if (200..300).contains(&status) {
                serde_json::from_str(&body).map_err(|e| TokenError::Temporary(format!("unreadable token answer ({status}): {e}")))
            } else if status >= 500 {
                Err(TokenError::Temporary(format!("token endpoint: HTTP {status}")))
            } else {
                let err: TokenErrorResponse = serde_json::from_str(&body)
                    .map_err(|_| TokenError::Temporary(format!("token endpoint: HTTP {status}: {body}")))?;
                Err(token_error_why(None, &err.error, err.error_description.as_deref().unwrap_or("")))
            }
        })
        .await
        .map_err(|e| TokenError::Temporary(format!("token endpoint task: {e}")))?
    }

    /// Exchanges the authorization code for the tokens, with the PKCE
    /// verifier, then checks the `id_token` (`oauth2.md`, section 5).
    async fn exchange(self: &Arc<Self>, flow_id: &str, code: &str) -> Result<FlowTokens, Why> {
        let (verifier, port, client_id, nonce, email) = {
            let inner = self.inner.lock().unwrap();
            let f = inner.flows.get(flow_id).ok_or_else(|| r::INVALID_PARAMS.with("flow", flow_id))?;
            (f.verifier.clone(), f.port, self.client_id().unwrap_or_default(), f.nonce.clone(), f.email.clone())
        };
        let redirect_uri = format!("http://127.0.0.1:{port}");
        let form = form_encode(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &redirect_uri),
            ("client_id", &client_id),
            ("code_verifier", &verifier),
        ]);
        let resp = self.token_post(&form).await.map_err(|e| match e {
            TokenError::Temporary(m) => r::IMAP_UNREACHABLE.with("server_text", m),
            TokenError::Reauth(w) | TokenError::Refused(w) => w,
        })?;
        let id_token = resp.id_token.ok_or_else(|| Why::from(r::OAUTH_STATE_MISMATCH))?;
        let claims = decode_id_token(&id_token).ok_or_else(|| Why::from(r::OAUTH_STATE_MISMATCH))?;
        // The nonce ties the id_token to this flow.
        if claims.nonce.as_deref() != Some(nonce.as_str()) {
            return Err(Why::from(r::OAUTH_STATE_MISMATCH));
        }
        // The user must have authorized the account that was asked for.
        let authorized = claims
            .email
            .or(claims.preferred_username)
            .ok_or_else(|| r::OAUTH_WRONG_ACCOUNT.with("authorized", "?").with("email", email.as_str()))?;
        if !authorized.eq_ignore_ascii_case(&email) {
            return Err(r::OAUTH_WRONG_ACCOUNT.with("authorized", authorized.as_str()).with("email", email.as_str()));
        }
        Ok(FlowTokens {
            access_token: SecretString::new(resp.access_token),
            access_expires: Instant::now() + Duration::from_secs(resp.expires_in.unwrap_or(3600)),
            refresh_token: SecretString::new(resp.refresh_token.unwrap_or_default()),
            client_id,
            tenant: claims.tid,
            user_id: claims.oid.or(claims.sub),
            email: authorized,
        })
    }

    /// The renewal grant (`oauth2.md`, section 6).
    async fn refresh(&self, cred: &OauthCredential, account: Option<&str>) -> Result<TokenResponse, TokenError> {
        let client_id = self.client_id().unwrap_or_else(|| cred.client_id.clone());
        let form = form_encode(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", cred.refresh_token.expose()),
            ("client_id", &client_id),
            ("scope", SCOPES),
        ]);
        // The account key, for the reasons that name it (owned: the
        // blocking call below outlives this method's borrows).
        let account = account.map(str::to_string);
        let url = self.token_url.lock().unwrap().clone();
        let https_only = *self.https_only.lock().unwrap();
        tokio::task::spawn_blocking(move || -> Result<TokenResponse, TokenError> {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(30)))
                .https_only(https_only)
                .max_redirects(0)
                .user_agent("NiceMailer")
                .http_status_as_error(false)
                .build()
                .into();
            let response = agent
                .post(&url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .send(&form)
                .map_err(|e| TokenError::Temporary(format!("token endpoint: {e}")))?;
            let status = response.status().as_u16();
            let body = response.into_body().read_to_string().map_err(|e| TokenError::Temporary(format!("token endpoint: {e}")))?;
            if (200..300).contains(&status) {
                serde_json::from_str(&body).map_err(|e| TokenError::Temporary(format!("unreadable token answer ({status}): {e}")))
            } else if status >= 500 {
                Err(TokenError::Temporary(format!("token endpoint: HTTP {status}")))
            } else {
                let err: TokenErrorResponse = serde_json::from_str(&body)
                    .map_err(|_| TokenError::Temporary(format!("token endpoint: HTTP {status}: {body}")))?;
                Err(token_error_why(account.as_deref(), &err.error, err.error_description.as_deref().unwrap_or("")))
            }
        })
        .await
        .map_err(|e| TokenError::Temporary(format!("token endpoint task: {e}")))?
    }

    /// The accept loop of one flow, then the code exchange, then the
    /// `OAuthFinished` event. One `GET /` ends the flow; any other
    /// request gets a 404 and the listener keeps waiting.
    async fn drive(self: Arc<Self>, flow_id: String, l4: TcpListener, l6: Option<TcpListener>, mut cancel_rx: tokio::sync::watch::Receiver<bool>) {
        let deadline = tokio::time::Instant::now() + FLOW_TIMEOUT;
        enum Outcome {
            Cancelled,
            Timeout,
            Denied,
            StateMismatch,
            Code(String),
        }
        let outcome = loop {
            let accept = tokio::select! {
                _ = cancel_rx.changed() => break Outcome::Cancelled,
                _ = tokio::time::sleep_until(deadline) => break Outcome::Timeout,
                a = l4.accept() => a.map(|(s, _)| s),
                a = async { match &l6 { Some(l) => l.accept().await.map(|(s, _)| s), None => std::future::pending().await } } => a,
            };
            let Ok(stream) = accept else { continue };
            match read_redirect(stream).await {
                Redirect::Params(params, accept_language, stream) => {
                    let state_ok = {
                        let inner = self.inner.lock().unwrap();
                        inner.flows.get(&flow_id).map(|f| f.state.as_str()) == params.get("state").map(String::as_str)
                    };
                    if !state_ok {
                        respond_page(stream, false, accept_language).await;
                        break Outcome::StateMismatch;
                    }
                    if params.contains_key("error") {
                        respond_page(stream, false, accept_language).await;
                        break Outcome::Denied;
                    }
                    if let Some(code) = params.get("code") {
                        respond_page(stream, true, accept_language).await;
                        break Outcome::Code(code.clone());
                    }
                    // `GET /` without code nor error: not this flow's
                    // redirection, keep listening.
                    respond_404(stream).await;
                }
                Redirect::Other(stream) => respond_404(stream).await,
                Redirect::Closed(stream) => drop(stream),
            }
        };
        let result = match outcome {
            Outcome::Cancelled => Err(r::OAUTH_CANCELLED.into()),
            Outcome::Timeout => Err(r::OAUTH_TIMEOUT.into()),
            Outcome::Denied => Err(r::OAUTH_DENIED.into()),
            Outcome::StateMismatch => Err(r::OAUTH_STATE_MISMATCH.into()),
            Outcome::Code(code) => self.exchange(&flow_id, &code).await,
        };
        self.finish_flow(&flow_id, result);
    }

    /// Records the end of a flow and emits `OAuthFinished`.
    fn finish_flow(&self, flow_id: &str, result: Result<FlowTokens, Why>) {
        let (email, account) = {
            let mut inner = self.inner.lock().unwrap();
            let Some(f) = inner.flows.get_mut(flow_id) else { return };
            if let Ok(t) = &result {
                f.tokens = Some(t.clone());
            }
            (f.email.clone(), f.account.clone())
        };
        let (ok, why) = match &result {
            Ok(_) => (true, None),
            Err(w) => (false, Some(w.data())),
        };
        self.emit(Event::OAuthFinished { flow: flow_id.to_string(), ok, account, email, why });
    }
}

/// Decodes the payload of an `id_token` (base64url JSON). The signature is
/// not checked: the token comes directly from the provider over TLS, in
/// answer to our own request, which OpenID Connect Core (3.1.3.7)
/// dispenses us from; the IMAP server decides the access anyway.
fn decode_id_token(id_token: &str) -> Option<IdTokenClaims> {
    let payload = id_token.split('.').nth(1)?;
    let json = b64url_decode(payload)?;
    serde_json::from_slice(&json).ok()
}

// ---------------------------------------------------------------------------
// The loopback listener
// ---------------------------------------------------------------------------

enum Redirect {
    /// A `GET /?…`: the redirection, with its query parameters, the
    /// `Accept-Language` header (for the language of the answer page) and
    /// the stream, to answer on.
    Params(BTreeMap<String, String>, Option<String>, TcpStream),
    /// Any other request: gets a 404, does not end the flow.
    Other(TcpStream),
    /// The connection ended or was too slow: just close it.
    Closed(TcpStream),
}

/// Reads one request, with the limits of `oauth2.md` section 5: at most
/// 8 KiB of headers, in 5 seconds at most, loopback only (the listener is
/// bound to 127.0.0.1 and [::1]).
async fn read_redirect(mut stream: TcpStream) -> Redirect {
    let mut buf: Vec<u8> = Vec::with_capacity(512);
    let read = tokio::time::timeout(READ_TIMEOUT, async {
        loop {
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                return Ok(());
            }
            if buf.len() > MAX_HEADERS {
                return Err(());
            }
            let mut chunk = [0u8; 1024];
            let n = stream.read(&mut chunk).await.map_err(|_| ())?;
            if n == 0 {
                return Err(());
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    })
    .await;
    if read.is_err() {
        return Redirect::Closed(stream);
    }
    let head = String::from_utf8_lossy(&buf);
    let request_line = head.lines().next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if method != "GET" {
        return Redirect::Other(stream);
    }
    let Some((path, query)) = target.split_once('?') else {
        return Redirect::Other(stream);
    };
    if path != "/" {
        return Redirect::Other(stream);
    }
    // The `Accept-Language` header decides the language of the page.
    let mut accept_language = None;
    for line in head.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') && name.trim().eq_ignore_ascii_case("accept-language") {
            accept_language = Some(value.trim().to_string());
        }
    }
    Redirect::Params(query_params(query), accept_language, stream)
}

/// The short page the browser gets: no external resource, English or
/// French by the `Accept-Language` header, English by default.
async fn respond_page(mut stream: TcpStream, ok: bool, accept_language: Option<String>) {
    let fr = accept_language.as_deref().unwrap_or("").to_ascii_lowercase().contains("fr");
    let (title, body) = if fr {
        if ok { ("Connexion terminée", "La connexion a réussi. Vous pouvez fermer cet onglet.") } else { ("Échec de la connexion", "La connexion a échoué. Vous pouvez fermer cet onglet.") }
    } else {
        if ok { ("Sign-in complete", "The sign-in succeeded. You can close this tab.") } else { ("Sign-in failed", "The sign-in failed. You can close this tab.") }
    };
    let page = format!("<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head><body><p>{body}</p></body></html>");
    let head = format!("HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", page.len());
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(page.as_bytes()).await;
}

async fn respond_404(mut stream: TcpStream) {
    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64url_round_trips() {
        let v: [u8; 32] = rand::random();
        let s = b64url(&v);
        assert_eq!(b64url_decode(&s).as_deref(), Some(&v[..]));
        assert!(!s.contains('+') && !s.contains('/') && !s.contains('='));
    }

    #[test]
    fn pkce_challenge_is_sha256_of_the_verifier() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn percent_encoding_and_query_parsing() {
        assert_eq!(pct("a b+c%d"), "a%20b%2Bc%25d");
        let q = query_params("code=abc%20def&state=x+y&error=");
        assert_eq!(q.get("code").unwrap(), "abc def");
        assert_eq!(q.get("state").unwrap(), "x y");
        assert_eq!(q.get("error").unwrap(), "");
    }

    #[test]
    fn id_token_payload_is_decoded_without_checking_the_signature() {
        // header.payload.signature, payload = {"nonce":"n","email":"a@b.c","tid":"t","oid":"o"}
        let payload = b64url(br#"{"nonce":"n","email":"a@b.c","tid":"t","oid":"o"}"#);
        let claims = decode_id_token(&format!("aaa.{payload}.ccc")).unwrap();
        assert_eq!(claims.nonce.as_deref(), Some("n"));
        assert_eq!(claims.email.as_deref(), Some("a@b.c"));
        assert_eq!(claims.tid.as_deref(), Some("t"));
        assert_eq!(claims.oid.as_deref(), Some("o"));
    }

    #[test]
    fn token_errors_map_to_the_reasons_of_the_spec() {
        let e = token_error_why(Some("k"), "invalid_grant", "the token expired");
        assert!(matches!(e, TokenError::Reauth(w) if w.reason.name == "oauth_reauth_needed" && w.values["account"] == "k"));
        let e = token_error_why(None, "invalid_scope", "AADSTS65001: consent required");
        assert!(matches!(e, TokenError::Refused(w) if w.reason.name == "oauth_admin_consent_needed" && w.values["system_text"].as_str().unwrap().contains("AADSTS65001")));
        let e = token_error_why(None, "x", "AADSTS53003: blocked by conditional access");
        assert!(matches!(e, TokenError::Refused(w) if w.reason.name == "oauth_blocked_by_policy"));
        let e = token_error_why(None, "invalid_client", "bad id");
        assert!(matches!(e, TokenError::Refused(w) if w.reason.name == "oauth_client_rejected"));
        let e = token_error_why(None, "temporarily_unavailable", "down");
        assert!(e.is_temporary());
    }

    #[test]
    fn the_provider_table_is_compiled_in() {
        assert_eq!(provider_for_imap("outlook.office365.com").map(|p| p.name), Some("microsoft"));
        assert_eq!(provider_for_imap("Outlook.Office365.COM").map(|p| p.name), Some("microsoft"));
        assert!(provider_for_imap("imap.gmail.com").is_none());
        assert_eq!(provider_for_smtp("smtp-mail.outlook.com").map(|p| p.name), Some("microsoft"));
        assert!(provider_for_smtp("outlook.office365.com").is_none(), "an IMAP host is not an SMTP host");
    }

    // --- a whole flow against a fake provider ------------------------------
    //
    // The token endpoint is a listener on the loopback; the browser is
    // scripted: one `GET /?…` on the flow's listener. Never Microsoft.

    use crate::secrets::MemorySecrets;

    /// A fake token endpoint: records the POST bodies, answers with tokens
    /// whose `id_token` carries the flow's nonce (the test reads it in the
    /// authorization URL) and `id_token_email`.
    fn spawn_token_server(id_token_email: &str) -> (String, Arc<Mutex<String>>, tokio::sync::mpsc::Receiver<String>) {
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = std_listener.local_addr().unwrap().port();
        std_listener.set_nonblocking(true).unwrap();
        let listener = TcpListener::from_std(std_listener).unwrap();
        let nonce = Arc::new(Mutex::new(String::new()));
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let email = id_token_email.to_string();
        let used: Arc<Mutex<std::collections::BTreeSet<String>>> = Arc::default();
        tokio::spawn({
            let nonce = nonce.clone();
            async move {
                loop {
                    let Ok((mut s, _)) = listener.accept().await else { break };
                    tokio::spawn({
                        let tx = tx.clone();
                        let nonce = nonce.clone();
                        let used = used.clone();
                        let email = email.clone();
                        async move {
                            let mut buf: Vec<u8> = Vec::new();
                            let mut chunk = [0u8; 4096];
                            let head_end = loop {
                                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                    break p + 4;
                                }
                                match s.read(&mut chunk).await {
                                    Ok(0) | Err(_) => return,
                                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                                }
                            };
                            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                            let len: usize = head
                                .lines()
                                .skip(1)
                                .find_map(|l| {
                                    l.split_once(':')
                                        .filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
                                        .and_then(|(_, v)| v.trim().parse().ok())
                                })
                                .unwrap_or(0);
                            let mut body = buf[head_end..].to_vec();
                            while body.len() < len {
                                match s.read(&mut chunk).await {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => body.extend_from_slice(&chunk[..n]),
                                }
                            }
                            let body = String::from_utf8_lossy(&body).to_string();
                            let _ = tx.send(body.clone()).await;
                            let nonce = nonce.lock().unwrap().clone();
                            let id_token = format!("h.{}.s", b64url(format!(r#"{{"nonce":"{nonce}","email":"{email}","tid":"tenant-1","oid":"user-1"}}"#).as_bytes()));
                            let (status, answer) = if body.contains("grant_type=authorization_code") {
                                (200u16, format!(r#"{{"token_type":"Bearer","expires_in":3600,"access_token":"AT-from-code","refresh_token":"RT-1","id_token":"{id_token}"}}"#))
                            } else if body.contains("grant_type=refresh_token") {
                                // Rotation as Microsoft may do it: RT-n gives
                                // RT-(n+1), and a refresh token already
                                // presented once is invalid.
                                let rt = body.split('&').find_map(|kv| kv.strip_prefix("refresh_token=")).unwrap_or("").to_string();
                                let fresh = used.lock().unwrap().insert(rt.clone());
                                match rt.strip_prefix("RT-").and_then(|n| n.parse::<u32>().ok()) {
                                    Some(n) if fresh => (200, format!(r#"{{"token_type":"Bearer","expires_in":3600,"access_token":"AT-refreshed","refresh_token":"RT-{}"}}"#, n + 1)),
                                    _ => (400, r#"{"error":"invalid_grant","error_description":"AADSTS70000: the refresh token was already used"}"#.to_string()),
                                }
                            } else {
                                (400, r#"{"error":"invalid_request","error_description":"no"}"#.to_string())
                            };
                            let resp = format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}", answer.len());
                            let _ = s.write_all(resp.as_bytes()).await;
                        }
                    });
                }
            }
        });
        (format!("http://127.0.0.1:{port}/token"), nonce, rx)
    }

    struct Harness {
        oauth: Arc<Oauth>,
        events: Arc<Mutex<Vec<Event>>>,
        bodies: tokio::sync::mpsc::Receiver<String>,
        nonce: Arc<Mutex<String>>,
    }

    async fn harness(id_token_email: &str) -> Harness {
        let oauth = Arc::new(Oauth::new());
        oauth.set_client_id(Some("11111111-2222-3333-4444-555555555555".to_string()));
        let (token_url, nonce, bodies) = spawn_token_server(id_token_email);
        oauth.set_endpoints("http://127.0.0.1:9/authorize", &token_url, false);
        let events = Arc::new(Mutex::new(Vec::new()));
        oauth.set_sink({
            let events = events.clone();
            Arc::new(move |ev| events.lock().unwrap().push(ev))
        });
        Harness { oauth, events, bodies, nonce }
    }

    /// `oauth_begin`, then the flow's `state`, its loopback port, and the
    /// nonce the fake token endpoint must put in the `id_token`.
    async fn begin_flow(h: &Harness, email: &str) -> (OauthBegin, String, u16) {
        let b = h.oauth.begin(email, None).await.unwrap();
        let q = query_params(b.url.split_once('?').unwrap().1);
        *h.nonce.lock().unwrap() = q.get("nonce").unwrap().clone();
        let state = q.get("state").unwrap().clone();
        let port = q.get("redirect_uri").unwrap().rsplit(':').next().unwrap().parse().unwrap();
        (b, state, port)
    }

    /// The scripted browser: one `GET /?…` on the flow's listener, and the
    /// answer page it got.
    async fn redirect(port: u16, query: &str, accept_language: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(format!("GET /?{query} HTTP/1.1\r\nhost: 127.0.0.1\r\naccept-language: {accept_language}\r\n\r\n").as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            match s.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => resp.extend_from_slice(&chunk[..n]),
            }
        }
        String::from_utf8_lossy(&resp).into_owned()
    }

    /// The next `OAuthFinished` event, in time.
    async fn finished(h: &Harness) -> Event {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(ev) = h.events.lock().unwrap().pop() {
                    break ev;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("an OAuthFinished event in time")
    }

    fn reason_of(ev: Event) -> (bool, Option<serde_json::Value>) {
        let Event::OAuthFinished { ok, why, .. } = ev else { panic!("not an OAuthFinished") };
        (ok, why)
    }

    #[tokio::test]
    async fn a_full_flow_from_the_url_to_the_stored_tokens() {
        let mut h = harness("me@outlook.com").await;
        let (b, state, port) = begin_flow(&h, "me@outlook.com").await;
        assert_eq!(b.provider, "microsoft");
        assert!(b.url.contains("code_challenge=") && b.url.contains("code_challenge_method=S256") && b.url.contains("prompt=select_account"));
        // The mail permissions of Exchange Online, named with their
        // resource: bare, they would be Microsoft Graph's.
        assert!(b.url.contains("scope=https%3A%2F%2Foutlook.office.com%2FIMAP.AccessAsUser.All"), "{}", b.url);
        assert!(b.url.contains("https%3A%2F%2Foutlook.office.com%2FSMTP.Send"), "{}", b.url);
        // The browser is redirected, in French this time.
        let page = redirect(port, &format!("code=the-code&state={state}"), "fr-FR").await;
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(page.contains("Connexion terminée"), "the page follows Accept-Language: {page}");
        let (ok, why) = reason_of(finished(&h).await);
        assert!(ok && why.is_none());
        // The code exchange carried the PKCE verifier and the redirect URI.
        let body = h.bodies.recv().await.unwrap();
        assert!(body.contains("grant_type=authorization_code"), "{body}");
        assert!(body.contains("code=the-code") && body.contains("code_verifier="), "{body}");
        assert!(body.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A"), "{body}");
        // `take_flow` gives the tokens, to the account asked for only.
        let tokens = h.oauth.take_flow(&b.flow, "me@outlook.com").unwrap();
        assert_eq!(tokens.access_token.expose(), "AT-from-code");
        assert_eq!(tokens.refresh_token.expose(), "RT-1");
        assert_eq!(tokens.tenant.as_deref(), Some("tenant-1"));
        assert!(h.oauth.take_flow(&b.flow, "other@outlook.com").is_err());
        // `store_flow` keeps the refresh token in the secrets, bound to the
        // server and user name of the file; the access token stays in memory.
        let secrets = MemorySecrets::default();
        h.oauth.store_flow("me", "me@outlook.com", "outlook.office365.com", "me@outlook.com", &secrets, &b.flow).unwrap();
        let cred = secrets.oauth("me").unwrap().unwrap();
        assert_eq!(cred.refresh_token.expose(), "RT-1");
        assert_eq!(cred.binding.host, "outlook.office365.com");
        let accounts = crate::config::parse_accounts(
            "[accounts.me]\nemail = \"me@outlook.com\"\nauth = \"oauth2\"\n[accounts.me.imap]\nhost = \"outlook.office365.com\"\nusername = \"me@outlook.com\"\n",
        )
        .unwrap();
        let cfg = &accounts["me"];
        assert_eq!(cfg.auth, AuthMethod::Oauth2);
        // While the token is valid, no call to the provider.
        let t = h.oauth.access_token("me", cfg, &secrets).await.unwrap();
        assert_eq!(t.expose(), "AT-from-code");
        assert!(h.bodies.try_recv().is_err(), "no renewal of a valid token");
        // A forced renewal goes to the token endpoint and rotates the
        // refresh token, stored before the access token is used.
        let t = h.oauth.access_token_forced("me", cfg, &secrets).await.unwrap();
        assert_eq!(t.expose(), "AT-refreshed");
        let body = h.bodies.recv().await.unwrap();
        assert!(body.contains("grant_type=refresh_token") && body.contains("refresh_token=RT-1"), "{body}");
        assert_eq!(secrets.oauth("me").unwrap().unwrap().refresh_token.expose(), "RT-2");
    }

    #[tokio::test]
    async fn a_denied_authorization_ends_the_flow_with_oauth_denied() {
        let h = harness("me@outlook.com").await;
        let (b, state, port) = begin_flow(&h, "me@outlook.com").await;
        let page = redirect(port, &format!("error=access_denied&state={state}"), "en").await;
        assert!(page.contains("Sign-in failed"), "{page}");
        let (ok, why) = reason_of(finished(&h).await);
        assert!(!ok);
        assert_eq!(why.unwrap()["reason"], "oauth_denied");
        assert!(h.oauth.take_flow(&b.flow, "me@outlook.com").is_err());
    }

    #[tokio::test]
    async fn a_state_that_does_not_match_ends_the_flow() {
        let h = harness("me@outlook.com").await;
        let (_b, _state, port) = begin_flow(&h, "me@outlook.com").await;
        redirect(port, "code=x&state=tampered", "en").await;
        let (ok, why) = reason_of(finished(&h).await);
        assert!(!ok);
        assert_eq!(why.unwrap()["reason"], "oauth_state_mismatch");
    }

    #[tokio::test]
    async fn another_account_than_the_one_asked_for_is_refused() {
        let h = harness("someone.else@outlook.com").await;
        let (b, state, port) = begin_flow(&h, "me@outlook.com").await;
        redirect(port, &format!("code=c&state={state}"), "en").await;
        let (ok, why) = reason_of(finished(&h).await);
        assert!(!ok);
        let why = why.unwrap();
        assert_eq!(why["reason"], "oauth_wrong_account");
        assert_eq!(why["authorized"], "someone.else@outlook.com");
        assert_eq!(why["email"], "me@outlook.com");
        assert!(h.oauth.take_flow(&b.flow, "me@outlook.com").is_err());
    }

    #[tokio::test]
    async fn cancel_closes_the_flow_with_oauth_cancelled() {
        let h = harness("me@outlook.com").await;
        let (b, _state, _port) = begin_flow(&h, "me@outlook.com").await;
        h.oauth.cancel(&b.flow);
        let (ok, why) = reason_of(finished(&h).await);
        assert!(!ok);
        assert_eq!(why.unwrap()["reason"], "oauth_cancelled");
    }

    #[tokio::test]
    async fn a_new_flow_for_the_same_address_cancels_the_previous_one() {
        let h = harness("me@outlook.com").await;
        let (first, _s, _p) = begin_flow(&h, "me@outlook.com").await;
        let (_second, _s, _p) = begin_flow(&h, "me@outlook.com").await;
        let ev = finished(&h).await;
        let Event::OAuthFinished { flow, .. } = &ev else { panic!() };
        assert_eq!(*flow, first.flow, "the cancelled flow announced itself");
        let (ok, why) = reason_of(ev);
        assert!(!ok);
        assert_eq!(why.unwrap()["reason"], "oauth_cancelled");
    }

    /// Two new accounts signed in at once (the CLI and the app): neither
    /// flow has an account key, and one must not cancel the other.
    #[tokio::test]
    async fn flows_for_two_new_addresses_do_not_cancel_each_other() {
        let h = harness("a@outlook.com").await;
        let (first, _s, _p) = begin_flow(&h, "a@outlook.com").await;
        let (_second, _s, _p) = begin_flow(&h, "b@outlook.com").await;
        assert!(h.events.lock().unwrap().is_empty(), "no flow was cancelled: {:?}", h.events.lock().unwrap());
        assert!(h.oauth.inner.lock().unwrap().flows.contains_key(&first.flow));
    }

    /// Secrets whose OAuth2 item can be made unwritable, as a locked
    /// Keychain is.
    #[derive(Default)]
    struct LockableSecrets {
        inner: MemorySecrets,
        locked: std::sync::atomic::AtomicBool,
    }

    impl Secrets for LockableSecrets {
        fn imap_binding(&self, key: &str) -> Result<Option<Binding>, String> {
            self.inner.imap_binding(key)
        }
        fn imap(&self, key: &str) -> Result<Option<crate::secrets::Credential>, String> {
            self.inner.imap(key)
        }
        fn set_imap(&self, key: &str, c: &crate::secrets::Credential) -> Result<(), String> {
            self.inner.set_imap(key, c)
        }
        fn remove_imap(&self, key: &str) -> Result<(), String> {
            self.inner.remove_imap(key)
        }
        fn smtp(&self, key: &str) -> Result<Option<crate::secrets::Credential>, String> {
            self.inner.smtp(key)
        }
        fn set_smtp(&self, key: &str, c: &crate::secrets::Credential) -> Result<(), String> {
            self.inner.set_smtp(key, c)
        }
        fn remove_smtp(&self, key: &str) -> Result<(), String> {
            self.inner.remove_smtp(key)
        }
        fn oauth(&self, key: &str) -> Result<Option<OauthCredential>, String> {
            self.inner.oauth(key)
        }
        fn set_oauth(&self, key: &str, c: &OauthCredential) -> Result<(), String> {
            if self.locked.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("the Keychain is locked".into());
            }
            self.inner.set_oauth(key, c)
        }
    }

    /// A rotated refresh token the Keychain refuses is kept in memory and
    /// presented at the next renewal, then written (`oauth2.md`, section
    /// 6): the old one, still in the Keychain, may be invalid already.
    #[tokio::test]
    async fn a_rotated_refresh_token_the_keychain_refuses_is_not_lost() {
        let mut h = harness("me@outlook.com").await;
        let (b, state, port) = begin_flow(&h, "me@outlook.com").await;
        redirect(port, &format!("code=the-code&state={state}"), "en").await;
        finished(&h).await;
        h.bodies.recv().await.unwrap();
        let secrets = LockableSecrets::default();
        h.oauth.store_flow("me", "me@outlook.com", "outlook.office365.com", "me@outlook.com", &secrets, &b.flow).unwrap();
        let accounts = crate::config::parse_accounts(
            "[accounts.me]\nemail = \"me@outlook.com\"\nauth = \"oauth2\"\n[accounts.me.imap]\nhost = \"outlook.office365.com\"\nusername = \"me@outlook.com\"\n",
        )
        .unwrap();
        let cfg = &accounts["me"];

        // Locked: RT-1 is rotated to RT-2, which cannot be written.
        secrets.locked.store(true, std::sync::atomic::Ordering::SeqCst);
        h.oauth.access_token_forced("me", cfg, &secrets).await.unwrap();
        assert!(h.bodies.recv().await.unwrap().contains("refresh_token=RT-1"));
        assert_eq!(secrets.oauth("me").unwrap().unwrap().refresh_token.expose(), "RT-1");

        // Unlocked: the next renewal presents RT-2, not the used RT-1 the
        // Keychain still holds, and writes its successor.
        secrets.locked.store(false, std::sync::atomic::Ordering::SeqCst);
        let t = h.oauth.access_token_forced("me", cfg, &secrets).await.unwrap();
        assert_eq!(t.expose(), "AT-refreshed");
        assert!(h.bodies.recv().await.unwrap().contains("refresh_token=RT-2"));
        assert_eq!(secrets.oauth("me").unwrap().unwrap().refresh_token.expose(), "RT-3");
        assert!(h.oauth.inner.lock().unwrap().unsaved_refresh.is_empty());
    }

    #[tokio::test]
    async fn an_unsupported_provider_is_refused_before_listening() {
        let h = harness("me@gmail.com").await;
        let err = h.oauth.begin("me@gmail.com", None).await.map(|_| ()).unwrap_err();
        assert_eq!(err.reason.name, "oauth2_unsupported_provider");
        assert_eq!(err.values["email"], "me@gmail.com");
    }

    #[tokio::test]
    async fn no_usable_client_id_is_a_clean_refusal() {
        // No id in effect (a development build without
        // NICEMAILER_MICROSOFT_CLIENT_ID, no config override): `oauth_begin`
        // says so without listening.
        let oauth = Arc::new(Oauth::new());
        oauth.set_client_id(None);
        let err = oauth.begin("me@outlook.com", None).await.map(|_| ()).unwrap_err();
        assert_eq!(err.reason.name, "oauth_client_rejected");
    }

    #[test]
    fn compiled_client_id_is_a_zero_or_valid_guid() {
        // Whatever the build embedded: the zero GUID (development without
        // NICEMAILER_MICROSOFT_CLIENT_ID) or a non-zero GUID, a form checked
        // by build.rs when the variable is set.
        assert!(
            COMPILED_CLIENT_ID == "00000000-0000-0000-0000-000000000000" || crate::config::is_client_id(COMPILED_CLIENT_ID),
            "the compiled client id is neither the zero GUID nor a valid one: {COMPILED_CLIENT_ID:?}"
        );
    }
}
