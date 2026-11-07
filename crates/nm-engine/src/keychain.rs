//! The macOS Keychain as the store of secrets ([`Secrets`]).
//!
//! # One item per account
//!
//! A *generic password* item of the login keychain:
//!
//! | Attribute | Value |
//! | --- | --- |
//! | service | `NiceMailer` |
//! | account | `imap:<key>` (the key of `[accounts.<key>]`) |
//! | label | `NiceMailer: <key> (IMAP)`, what Keychain Access shows |
//! | comment | `{"host": "…", "username": "…"}`: the server it was saved for |
//! | data | the password, UTF-8 |
//!
//! The binding to the server is in the comment, an attribute: the Keychain
//! gives attributes to anyone without asking, while the data needs the
//! item's permission (see `secrets.rs`, "Two reads").
//!
//! # Who may read the password
//!
//! An item of the file keychain has an access list: the application that
//! created it reads it without any dialog, any other one needs the user's
//! permission. macOS recognizes the application by its code signature.
//! The core creates the items itself (the CLI and the app send it the
//! password), so the core, and only it, reads them.
//!
//! A Rust binary is signed by the linker with a signature of its own
//! content ("ad hoc"): every rebuild is a new application for the Keychain.
//! In development, `make core` therefore signs the core with an Apple
//! Development certificate (`scripts/sign-core.sh`). Its Team ID matters:
//! besides the access list, an item of the login keychain has a
//! *partition* made from the signature of its creator, `teamid:<team>`
//! for a signature with a Team ID, the same from one build to the next,
//! but `cdhash:<hash of the binary>` for one without (a self-signed
//! certificate), which the next build does not match
//! (`docs/configuration.md`, "Passwords"). The shipped core is signed by
//! the developer ID of the same team, which plays the same role.
//!
//! # Never a dialog
//!
//! The core runs in the background: a Keychain dialog would appear out of
//! nowhere and block the thread that asked, with the core's locks held.
//! [`never_prompt`] turns them off for the whole process at start, so an
//! access that would need one fails instead, and the account's error says
//! why.

use core_foundation::data::CFData;
use security_framework::base::Error;
use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass, ItemSearchOptions, Location, SearchResult};
use security_framework::os::macos::keychain::SecKeychain;

use crate::config::SecretString;
use crate::secrets::{Binding, Credential, Secrets};

const SERVICE: &str = "NiceMailer";

// Status codes of the Security framework (`SecBase.h`).
/// errSecItemNotFound.
const NOT_FOUND: i32 = -25300;
/// errSecDuplicateItem.
const DUPLICATE: i32 = -25299;
/// errSecAuthFailed: the access list refused, and no dialog was allowed.
const AUTH_FAILED: i32 = -25293;
/// errSecInteractionNotAllowed: a dialog was needed (locked keychain…).
const INTERACTION_NOT_ALLOWED: i32 = -25308;

/// Turns the Keychain's dialogs off for the whole process, for good.
///
/// Rust note: `disable_user_interaction` returns a guard that turns them
/// back on when dropped (`Drop`). `std::mem::forget` drops nothing and
/// runs no destructor: the guard is never dropped, so the setting stays.
pub fn never_prompt() -> Result<(), String> {
    let guard = SecKeychain::disable_user_interaction().map_err(|e| e.to_string())?;
    std::mem::forget(guard);
    Ok(())
}

/// The secrets of the accounts in a keychain: the login keychain, or a
/// keychain file of its own (tests, a separate development keychain).
pub struct Keychain {
    /// `None`: the default keychain, the login keychain.
    file: Option<SecKeychain>,
}

impl Keychain {
    /// The login keychain.
    pub fn login() -> Keychain {
        Keychain { file: None }
    }

    /// A keychain file, which must exist and be unlocked
    /// (`security create-keychain`, `security unlock-keychain`).
    pub fn file(path: &std::path::Path) -> Result<Keychain, String> {
        let kc = SecKeychain::open(path).map_err(|e| format!("keychain {}: {e}", path.display()))?;
        Ok(Keychain { file: Some(kc) })
    }

    /// The account name of an item: `imap:<key>`, or `smtp:<key>` for the
    /// SMTP password saved apart (`reponse.md`, section 7).
    fn account(proto: Proto, key: &str) -> String {
        format!("{}:{key}", proto.prefix())
    }

    /// A search for the item of `key`.
    fn search(&self, proto: Proto, key: &str) -> ItemSearchOptions {
        let mut s = ItemSearchOptions::new();
        s.class(ItemClass::generic_password()).service(SERVICE).account(&Self::account(proto, key));
        if let Some(kc) = &self.file {
            s.keychains(std::slice::from_ref(kc));
        }
        s
    }

    /// The attributes (and the data, if `data`) of the item of `key`, as
    /// strings by attribute name (`icmt` comment, `v_Data` data).
    fn find(&self, proto: Proto, key: &str, data: bool) -> Result<Option<std::collections::HashMap<String, String>>, String> {
        let mut s = self.search(proto, key);
        s.load_attributes(true).load_data(data);
        match s.search() {
            Ok(items) => Ok(items.first().and_then(SearchResult::simplify_dict)),
            Err(e) if e.code() == NOT_FOUND => Ok(None),
            Err(e) => Err(describe(e)),
        }
    }
}

/// The comment holding the server: `{"host": …, "username": …}`.
fn binding_of(attrs: &std::collections::HashMap<String, String>) -> Result<Binding, String> {
    let comment = attrs.get("icmt").ok_or("Keychain item without its server: save the password again")?;
    let v: serde_json::Value = serde_json::from_str(comment).map_err(|_| "Keychain item with an unreadable server: save the password again")?;
    match (v["host"].as_str(), v["username"].as_str()) {
        (Some(h), Some(u)) => Ok(Binding { host: h.to_string(), username: u.to_string() }),
        _ => Err("Keychain item with an unreadable server: save the password again".into()),
    }
}

/// A message for the user about a failure of the Keychain.
fn describe(e: Error) -> String {
    match e.code() {
        AUTH_FAILED | INTERACTION_NOT_ALLOWED => "the Keychain refused the password of this account: it is locked, \
             or the password was saved by another build of the core (sign it with an Apple Development certificate, \
             see docs/configuration.md, \"Passwords\", then save the password again)"
            .into(),
        _ => format!("Keychain: {e}"),
    }
}

/// Which password of an account an item holds.
#[derive(Debug, Clone, Copy)]
enum Proto {
    Imap,
    Smtp,
}

impl Proto {
    fn prefix(self) -> &'static str {
        match self {
            Proto::Imap => "imap",
            Proto::Smtp => "smtp",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Proto::Imap => "IMAP",
            Proto::Smtp => "SMTP",
        }
    }
}

impl Keychain {
    fn remove(&self, proto: Proto, key: &str) -> Result<(), String> {
        match self.search(proto, key).delete() {
            Ok(()) => Ok(()),
            Err(e) if e.code() == NOT_FOUND => Ok(()),
            Err(e) => Err(describe(e)),
        }
    }

    fn get(&self, proto: Proto, key: &str) -> Result<Option<Credential>, String> {
        let Some(attrs) = self.find(proto, key, true)? else { return Ok(None) };
        let binding = binding_of(&attrs)?;
        let secret = attrs.get("v_Data").ok_or("Keychain item without a password: save it again")?;
        Ok(Some(Credential { binding, secret: SecretString::new(secret.clone()) }))
    }

    fn set(&self, proto: Proto, key: &str, credential: &Credential) -> Result<(), String> {
        // Replace: delete the old item, then add the new one. Deleting
        // needs the item's permission, which the core has for the items it
        // created; an item saved by another build is refused, and the
        // message says what to do.
        match self.search(proto, key).delete() {
            Ok(()) => {}
            Err(e) if e.code() == NOT_FOUND => {}
            Err(e) => return Err(describe(e)),
        }
        let comment = serde_json::json!({ "host": credential.binding.host, "username": credential.binding.username }).to_string();
        let data = CFData::from_buffer(credential.secret.expose().as_bytes());
        let mut add = ItemAddOptions::new(ItemAddValue::Data { class: ItemClass::generic_password(), data });
        add.set_service(SERVICE)
            .set_account_name(Self::account(proto, key))
            .set_label(format!("NiceMailer: {key} ({})", proto.label()))
            .set_comment(comment);
        add.set_location(match &self.file {
            Some(kc) => Location::FileKeychain(kc.clone()),
            None => Location::DefaultFileKeychain,
        });
        match add.add() {
            Ok(()) => Ok(()),
            Err(e) if e.code() == DUPLICATE => Err("the Keychain already has this item and refused to replace it".into()),
            Err(e) => Err(describe(e)),
        }
    }
}

impl Secrets for Keychain {
    fn remove_imap(&self, key: &str) -> Result<(), String> {
        self.remove(Proto::Imap, key)
    }

    fn imap_binding(&self, key: &str) -> Result<Option<Binding>, String> {
        self.find(Proto::Imap, key, false)?.map(|a| binding_of(&a)).transpose()
    }

    fn imap(&self, key: &str) -> Result<Option<Credential>, String> {
        self.get(Proto::Imap, key)
    }

    fn set_imap(&self, key: &str, credential: &Credential) -> Result<(), String> {
        self.set(Proto::Imap, key, credential)
    }

    fn smtp(&self, key: &str) -> Result<Option<Credential>, String> {
        self.get(Proto::Smtp, key)
    }

    fn set_smtp(&self, key: &str, credential: &Credential) -> Result<(), String> {
        self.set(Proto::Smtp, key, credential)
    }

    fn remove_smtp(&self, key: &str) -> Result<(), String> {
        self.remove(Proto::Smtp, key)
    }
}
