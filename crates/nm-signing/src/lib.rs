//! Code-signature checks of the program at the other end of a Unix socket
//! (`specifications/architecture.md`, section 9, "Vérification de la
//! signature"). Both ends use it:
//!
//! - the core verifies each client that connects (`nm-daemon`,
//!   `signing.rs`): **a client's identity is its signature**, never what it
//!   declares in `hello`;
//! - the CLI and the MCP server verify that the program serving the socket
//!   is our core before they send it anything (a password, a draft).
//!
//! The steps: the peer's audit token, recorded by the kernel at the connect
//! (`getsockopt(LOCAL_PEERTOKEN)`, preferred to the PID, which the system
//! may give to another process between the connect and the check), then
//! `SecCodeCopyGuestWithAttributes` and `SecCodeCheckValidity` against a
//! requirement built from **our own signature**: the team is read at run
//! time from the running binary (`SecCodeCopySelf`), never embedded at
//! build time, so that a build signed by any identity of the team — a
//! release, a development bundle, `make core` — demands its own team.
//!
//! The policy ([`Policy`]):
//!
//! - **relaxed** in a debug build, or with the `dev-unverified` feature —
//!   both decided *at compile time*, never by an environment variable,
//!   which would be a back door in production (`architecture.md`,
//!   section 9). Nothing is checked: every peer is let through
//!   unverified, and the core takes the source from `hello`;
//! - **strict** otherwise: an Apple-issued certificate of our team, one of
//!   the expected identifiers, the hardened runtime, no `get-task-allow`
//!   (a debugger could attach and drive it); and, when we are signed with a
//!   Developer ID ourselves, a Developer ID peer, so that a release refuses
//!   development builds of the same team;
//! - **fail-closed** when a strict build has no team (unsigned or ad hoc):
//!   every peer is refused.
//!
//! Not covered, by design: an older version of one of our binaries, signed
//! by the same team, satisfies the requirement (`architecture.md`,
//! section 9, "Menaces couvertes et non couvertes").

use std::ffi::c_void;
use std::io;
use std::os::unix::io::RawFd;
use std::str::FromStr;
use std::sync::OnceLock;

use core_foundation::base::{CFType, TCFType};
use core_foundation::data::CFData;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use security_framework::os::macos::code_signing::{Flags, GuestAttributes, SecCode, SecRequirement};

/// The identifier of the core's signature (`scripts/sign-core.sh`).
pub const CORE_IDENTIFIER: &str = "eu.nicemailer.core";
/// The identifier of the app's signature (`scripts/make-bundle.sh`).
pub const APP_IDENTIFIER: &str = "eu.nicemailer.NiceMailer";
/// The identifier of the CLI.
pub const CLI_IDENTIFIER: &str = "eu.nicemailer.cli";
/// The identifier of the TUI (not written yet).
pub const TUI_IDENTIFIER: &str = "eu.nicemailer.tui";
/// The identifier of the MCP server.
pub const MCP_IDENTIFIER: &str = "eu.nicemailer.mcp";
/// The identifier of the web server (not written yet).
pub const WEB_IDENTIFIER: &str = "eu.nicemailer.web";

/// The identifiers of the clients the core accepts (`docs/app-bundle.md`,
/// "Signature"; `architecture.md`, section 9).
pub const CLIENT_IDENTIFIERS: &[&str] = &[APP_IDENTIFIER, CLI_IDENTIFIER, TUI_IDENTIFIER, MCP_IDENTIFIER, WEB_IDENTIFIER];

/// An audit token: `audit_token_t`, eight `unsigned int` (mach/message.h).
pub type AuditToken = [u32; 8];

/// Whether this build relaxes the checks, decided at compile time.
pub const RELAXED: bool = cfg!(debug_assertions) || cfg!(feature = "dev-unverified");

/// What a build demands of its peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Policy {
    /// Debug build, or `dev-unverified`: nothing is checked, every peer is
    /// let through unverified ([`requirement`] still writes the identifier
    /// alone).
    Relaxed,
    /// Our team, and a Developer ID when we are signed with one.
    Strict { team: String, developer_id: bool },
    /// A strict build without a team of its own (unsigned, ad hoc): no peer
    /// can be verified, every one is refused.
    NoTeam,
}

/// The result of a check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The signature meets the requirement; its identifier.
    Verified(String),
    /// Relaxed build, no acceptable signature: let through, unverified.
    Unverified,
    /// Refused, with the reason for the log.
    Refused(String),
}

/// The policy of this build, from our own signature (read once).
pub fn policy() -> &'static Policy {
    static POLICY: OnceLock<Policy> = OnceLock::new();
    POLICY.get_or_init(|| {
        if RELAXED {
            return Policy::Relaxed;
        }
        let me = own_signature();
        match me.team {
            Some(team) => Policy::Strict { team, developer_id: me.developer_id },
            None => Policy::NoTeam,
        }
    })
}

/// The audit token of the peer of the connected Unix socket `fd`.
pub fn peer_audit_token(fd: RawFd) -> io::Result<AuditToken> {
    let mut token: AuditToken = [0; 8];
    let mut len = std::mem::size_of::<AuditToken>() as libc::socklen_t;
    // Rust note: `getsockopt` is a C call, hence `unsafe`: the compiler
    // cannot check that the buffer and its length match. They do: `len`
    // is the size of `token`. The level `SYSPROTO_LOCAL` (0 on Darwin)
    // carries the `LOCAL_*` options of a Unix socket (sys/un.h).
    let rc = unsafe { libc::getsockopt(fd, SYSPROTO_LOCAL, LOCAL_PEERTOKEN, token.as_mut_ptr().cast(), &mut len) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    if len as usize != std::mem::size_of::<AuditToken>() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("audit token of {len} bytes")));
    }
    Ok(token)
}

/// Checks the process of `token` against the policy of this build and
/// `identifiers`. Blocking (it may read the peer's binary from disk): an
/// async caller runs it with `spawn_blocking`.
pub fn verify(token: &AuditToken, identifiers: &[&str]) -> Verdict {
    let policy = policy();
    // A relaxed build checks nothing: its peers are unsigned development
    // binaries, the answer would be ignored anyway, and a check costs over
    // half a second in a debug build (measured; a few milliseconds in a
    // release one).
    if *policy == Policy::Relaxed {
        return Verdict::Unverified;
    }
    match check(policy, token, identifiers) {
        Ok(identifier) => Verdict::Verified(identifier),
        Err(why) => Verdict::Refused(why),
    }
}

/// Whether the process of `token` is our core, for a client about to talk
/// to it: `Ok` when it is, or when this build is relaxed; the reason
/// otherwise. A program listening on the socket in place of the core would
/// otherwise receive what the client sends — a password, a draft
/// (`architecture.md`, section 9). Blocking, a few milliseconds.
pub fn verify_core(token: &AuditToken) -> Result<(), String> {
    match verify(token, &[CORE_IDENTIFIER]) {
        Verdict::Verified(_) | Verdict::Unverified => Ok(()),
        Verdict::Refused(why) => Err(why),
    }
}

/// The check itself: `Ok(identifier)` when the peer meets the requirement.
fn check(policy: &Policy, token: &AuditToken, identifiers: &[&str]) -> Result<String, String> {
    let text = requirement(policy, identifiers).ok_or("this build has no team id of its own (fail-closed)")?;
    let requirement = SecRequirement::from_str(&text).map_err(|e| format!("invalid requirement {text:?}: {e}"))?;
    // The attributes {kSecGuestAttributeAudit: <the token as data>}.
    let bytes: Vec<u8> = token.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let data = CFData::from_buffer(&bytes);
    let mut attrs = GuestAttributes::new();
    attrs.set_audit_token(data.as_concrete_TypeRef());
    let guest = SecCode::copy_guest_with_attribues(None, &attrs, Flags::NONE).map_err(|e| format!("no code for the peer: {e}"))?;
    guest.check_validity(Flags::NONE, &requirement).map_err(|e| format!("signature refused: {e}"))?;
    let info = signing_information(&guest).ok_or("no signing information")?;
    let identifier = info.identifier.ok_or("no identifier")?;
    if !identifiers.contains(&identifier.as_str()) {
        return Err(format!("unexpected identifier {identifier}"));
    }
    // The hardened runtime cannot be written in a requirement: it is a
    // flag of the code directory. Without it, `DYLD_INSERT_LIBRARIES`
    // would load foreign code into a binary that passes the check.
    if matches!(policy, Policy::Strict { .. }) && info.flags & CS_RUNTIME == 0 {
        return Err(format!("{identifier} without the hardened runtime"));
    }
    Ok(identifier)
}

/// The requirement for `identifiers` under `policy`, in Apple's requirement
/// language (`man csreq`). `None` when nothing can be verified.
///
/// Note: the language has no `identifier in (…)`; a list is written
/// `(identifier "a" or identifier "b")`. Its negation is `!`, not `not`.
/// The tests compile every requirement written here.
pub fn requirement(policy: &Policy, identifiers: &[&str]) -> Option<String> {
    if identifiers.is_empty() {
        return None;
    }
    let ids = format!("({})", identifiers.iter().map(|i| format!("identifier \"{i}\"")).collect::<Vec<_>>().join(" or "));
    match policy {
        Policy::Relaxed => Some(ids),
        Policy::NoTeam => None,
        Policy::Strict { team, developer_id } => {
            // A team id is ten capitals and digits: anything else would be
            // written into the requirement as it is.
            if !is_team_id(team) {
                return None;
            }
            let mut r = format!("anchor apple generic and certificate leaf[subject.OU] = \"{team}\" and {ids}");
            if *developer_id {
                // The Developer ID intermediate and leaf markers: refuses an
                // Apple Development certificate of the same team.
                r.push_str(DEVELOPER_ID_MARKERS);
            }
            // A debugger could attach to a binary that allows it, and drive it.
            r.push_str(" and !(entitlement[\"com.apple.security.get-task-allow\"] exists)");
            Some(r)
        }
    }
}

/// The markers of a Developer ID signature: the extension of Apple's
/// Developer ID intermediate certificate, and of its leaf.
const DEVELOPER_ID_MARKERS: &str =
    " and certificate 1[field.1.2.840.113635.100.6.2.6] and certificate leaf[field.1.2.840.113635.100.6.1.13]";

/// Ten capital letters or digits, the form of an Apple team id.
pub fn is_team_id(s: &str) -> bool {
    s.len() == 10 && s.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// What this binary's own signature says.
#[derive(Debug, Clone, Default)]
pub struct OwnSignature {
    /// The team id, when the signature has one (not ad hoc).
    pub team: Option<String>,
    /// Whether it is a Developer ID signature.
    pub developer_id: bool,
}

/// Our own signature: the team, and whether it is a Developer ID.
pub fn own_signature() -> OwnSignature {
    let Ok(me) = SecCode::for_self(Flags::NONE) else { return OwnSignature::default() };
    let team = signing_information(&me).and_then(|i| i.team).filter(|t| is_team_id(t));
    let developer_id = SecRequirement::from_str(&format!("anchor apple generic{DEVELOPER_ID_MARKERS}"))
        .is_ok_and(|r| me.check_validity(Flags::NONE, &r).is_ok());
    OwnSignature { team, developer_id }
}

/// The part of the signing information the checks read.
struct Info {
    identifier: Option<String>,
    team: Option<String>,
    /// The code-directory flags (`kSecCodeInfoFlags`).
    flags: u32,
}

/// `SecCodeCopySigningInformation` of `code`, with the signing details
/// (the team id is only there with `kSecCSSigningInformation`).
fn signing_information(code: &SecCode) -> Option<Info> {
    let mut raw: CFDictionaryRef = std::ptr::null();
    // Rust note: a C call through a raw pointer, hence `unsafe`. On
    // success the dictionary is ours ("Copy" in the name: the create rule),
    // and `wrap_under_create_rule` releases it when `dict` goes out of
    // scope, whatever the branch.
    let rc = unsafe { SecCodeCopySigningInformation(code.as_concrete_TypeRef().cast(), K_SEC_CS_SIGNING_INFORMATION, &mut raw) };
    if rc != 0 || raw.is_null() {
        return None;
    }
    let dict: CFDictionary<CFString, CFType> = unsafe { CFDictionary::wrap_under_create_rule(raw) };
    // The keys are constants of the Security framework; the "get rule":
    // borrowed, not released by us.
    let key = |k: CFStringRef| unsafe { CFString::wrap_under_get_rule(k) };
    let string = |k: CFStringRef| dict.find(key(k)).and_then(|v| v.downcast::<CFString>()).map(|s| s.to_string());
    let flags = dict
        .find(key(unsafe { kSecCodeInfoFlags }))
        .and_then(|v| v.downcast::<CFNumber>())
        .and_then(|n| n.to_i64())
        .unwrap_or(0) as u32;
    Some(Info { identifier: string(unsafe { kSecCodeInfoIdentifier }), team: string(unsafe { kSecCodeInfoTeamIdentifier }), flags })
}

// --- The few C interfaces the crates do not wrap -------------------------

/// `SYSPROTO_LOCAL` on Darwin (sys/socket.h).
const SYSPROTO_LOCAL: libc::c_int = 0;
/// `LOCAL_PEERTOKEN` (sys/un.h): the peer's audit token.
const LOCAL_PEERTOKEN: libc::c_int = 0x006;
/// `kSecCSSigningInformation` (SecCode.h).
const K_SEC_CS_SIGNING_INFORMATION: u32 = 1 << 1;
/// `CS_RUNTIME` (cs_blobs.h): the hardened runtime.
pub const CS_RUNTIME: u32 = 0x0001_0000;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecCodeInfoIdentifier: CFStringRef;
    static kSecCodeInfoTeamIdentifier: CFStringRef;
    static kSecCodeInfoFlags: CFStringRef;
    fn SecCodeCopySigningInformation(code: *const c_void, flags: u32, information: *mut CFDictionaryRef) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::UnixStream;

    const TEAM: &str = "ABCDE12345";

    /// Every requirement this crate writes compiles: a syntax error would
    /// refuse every peer at run time (the branch first wrote
    /// `identifier in (…)`, which the language does not have).
    #[test]
    fn every_requirement_compiles() {
        let policies = [
            Policy::Relaxed,
            Policy::Strict { team: TEAM.into(), developer_id: false },
            Policy::Strict { team: TEAM.into(), developer_id: true },
        ];
        for p in &policies {
            for ids in [CLIENT_IDENTIFIERS, &[CORE_IDENTIFIER][..]] {
                let r = requirement(p, ids).unwrap();
                assert!(SecRequirement::from_str(&r).is_ok(), "{r}");
                for i in ids {
                    assert!(r.contains(&format!("identifier \"{i}\"")), "{r}");
                }
            }
        }
    }

    #[test]
    fn a_strict_requirement_names_the_team_and_refuses_a_debuggable_peer() {
        let r = requirement(&Policy::Strict { team: TEAM.into(), developer_id: false }, CLIENT_IDENTIFIERS).unwrap();
        assert!(r.starts_with(&format!("anchor apple generic and certificate leaf[subject.OU] = \"{TEAM}\"")), "{r}");
        assert!(r.contains("get-task-allow"), "{r}");
        assert!(!r.contains("6.1.13"), "only a Developer ID build demands a Developer ID peer: {r}");
        let d = requirement(&Policy::Strict { team: TEAM.into(), developer_id: true }, CLIENT_IDENTIFIERS).unwrap();
        assert!(d.contains("certificate leaf[field.1.2.840.113635.100.6.1.13]"), "{d}");
        // The relaxed one never demands the anchor.
        assert!(!requirement(&Policy::Relaxed, CLIENT_IDENTIFIERS).unwrap().contains("anchor"));
    }

    /// Fail-closed: no team, an invalid team, no identifier — no requirement.
    #[test]
    fn nothing_to_require_refuses() {
        assert_eq!(requirement(&Policy::NoTeam, CLIENT_IDENTIFIERS), None);
        assert_eq!(requirement(&Policy::Strict { team: "\" or true".into(), developer_id: false }, CLIENT_IDENTIFIERS), None);
        assert_eq!(requirement(&Policy::Relaxed, &[]), None);
        let token = [0u32; 8];
        assert!(check(&Policy::NoTeam, &token, CLIENT_IDENTIFIERS).is_err());
    }

    #[test]
    fn a_team_id_is_ten_capitals_or_digits() {
        assert!(is_team_id(TEAM));
        assert!(!is_team_id("abcde12345"));
        assert!(!is_team_id("ABCDE1234"));
        assert!(!is_team_id("ABCDE\"1234"));
    }

    /// The peer of a socket pair is this test binary itself: its token is
    /// read, and its (linker, ad hoc) signature is not one of ours.
    #[test]
    fn the_test_binary_is_not_one_of_ours() {
        let (a, _b) = UnixStream::pair().unwrap();
        let token = peer_audit_token(a.as_raw_fd()).expect("the audit token of the peer");
        assert_ne!(token, [0; 8]);
        let expected = if RELAXED { "unverified" } else { "refused" };
        let got = match verify(&token, CLIENT_IDENTIFIERS) {
            Verdict::Verified(i) => panic!("the test binary passed as {i}"),
            Verdict::Unverified => "unverified",
            Verdict::Refused(_) => "refused",
        };
        assert_eq!(got, expected);
        // Nor is it the core: refused, unless relaxed.
        assert_eq!(verify_core(&token).is_ok(), RELAXED);
        // Not a socket: an error, not a token.
        assert!(peer_audit_token(-1).is_err());
    }

    /// The test binary has no team (ad hoc): a strict build of it would be
    /// fail-closed.
    #[test]
    fn an_ad_hoc_binary_has_no_team() {
        let me = own_signature();
        assert_eq!(me.team, None);
        assert!(!me.developer_id);
        if !RELAXED {
            assert_eq!(*policy(), Policy::NoTeam);
        }
    }
}
/// An end-to-end check against a real signed client, run by hand
/// (`docs/testing.md`, "Code signature"): the client connects to a socket
/// this test listens on, and is verified under the strict policy of its
/// own team. Ignored by default: it needs a signed `nicemailer`.
#[cfg(test)]
mod signed {
    use super::*;
    use std::os::unix::io::AsRawFd;

    /// `NICEMAILER_TEST_SIGNED_CLI=<path of a signed nicemailer>
    /// cargo test -p nm-signing -- --ignored`.
    #[test]
    #[ignore]
    fn a_signed_client_of_ours_is_verified() {
        let cli = std::env::var("NICEMAILER_TEST_SIGNED_CLI").expect("NICEMAILER_TEST_SIGNED_CLI: the path of a signed nicemailer");
        // A short path: a socket path must stay under 104 bytes.
        let dir = tempfile_dir();
        let socket = dir.join("core.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let mut child = std::process::Command::new(&cli)
            .env("NICEMAILER_DATA_DIR", &dir)
            .arg("folders")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let (conn, _) = listener.accept().unwrap();
        let token = peer_audit_token(conn.as_raw_fd()).unwrap();
        let team = team_of(&cli);
        let start = std::time::Instant::now();
        let strict = Policy::Strict { team: team.clone(), developer_id: false };
        let identifier = check(&strict, &token, CLIENT_IDENTIFIERS).expect("the signed CLI meets the requirement of its team");
        eprintln!("verified {identifier} of team {team} in {:?}", start.elapsed());
        assert_eq!(identifier, CLI_IDENTIFIER);
        // Not the core: the same client fails the requirement of the core.
        assert!(check(&strict, &token, &[CORE_IDENTIFIER]).is_err());
        // Another team: refused.
        assert!(check(&Policy::Strict { team: "ZZZZZZZZZZ".into(), developer_id: false }, &token, CLIENT_IDENTIFIERS).is_err());
        drop(conn);
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The team of the binary at `path`, read by `codesign`.
    fn team_of(path: &str) -> String {
        let out = std::process::Command::new("codesign").args(["-dvv", path]).output().unwrap();
        let text = String::from_utf8_lossy(&out.stderr).into_owned();
        text.lines().find_map(|l| l.strip_prefix("TeamIdentifier=")).expect("a team id").to_string()
    }

    /// A new directory with a short path, under the target directory.
    fn tempfile_dir() -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/sig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
