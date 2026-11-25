//! Verification of the code signature of each client that connects to the
//! socket (`specifications/architecture.md`, section 9, "Vérification de la
//! signature"): the audit token of the peer — `getsockopt(LOCAL_PEERTOKEN)`,
//! preferred to the PID, which the system may reassign between the connect
//! and the check — then `SecCodeCopyGuestWithAttributes` and
//! `SecCodeCheckValidity` against a requirement. **The identity of a client
//! is its signature, never what it declares in `hello`**: each of our
//! binaries has its own identifier, which decides its rights (`rights.rs`).
//!
//! In a debug build the requirement is relaxed *at compile time*
//! (`cfg!(debug_assertions)`) to accept ad-hoc and unsigned development
//! binaries, so that `cargo run` works — never through an environment
//! variable, which would be a back door in production
//! (`architecture.md`, section 9).

use std::ffi::c_void;
use std::os::unix::io::AsRawFd;

use nm_core::Source;
use tokio::net::UnixStream;

/// The team id of our signing certificate, embedded at build time with
/// `option_env!` (`NICEMAILER_TEAM_ID=… make bundle`, `scripts/make-bundle.sh`).
/// Empty when the build had none: a release core then refuses every client
/// (fail-closed), a debug core does not use it.
pub(crate) const TEAM_ID: &str = match option_env!("NICEMAILER_TEAM_ID") {
    Some(t) => t,
    None => "",
};

/// The identifiers of our signed binaries (`docs/app-bundle.md`,
/// "Signature"; `architecture.md`, section 9).
const CLIENT_IDENTIFIERS: &[&str] = &[
    "eu.nicemailer.NiceMailer",
    "eu.nicemailer.cli",
    "eu.nicemailer.tui",
    "eu.nicemailer.mcp",
    "eu.nicemailer.web",
];

/// What the verification of a connection found.
pub(crate) enum Admission {
    /// The signature meets the requirement; the identifier says which of
    /// our binaries the client is, and that alone gives its rights.
    Verified(Source),
    /// Debug build, no acceptable signature (an unsigned `cargo run`
    /// binary): `hello` declares the source, as it always did.
    DevUnverified,
    /// The signature does not meet the requirement: close the connection
    /// and log the refusal (`architecture.md`, section 9).
    Refused,
}

/// The source of a signed client, from its identifier (`architecture.md`,
/// section 9, "Droits par client").
pub(crate) fn source_of_identifier(identifier: &str) -> Option<Source> {
    Some(match identifier {
        "eu.nicemailer.NiceMailer" => Source::App,
        "eu.nicemailer.cli" => Source::Cli,
        "eu.nicemailer.tui" => Source::Tui,
        "eu.nicemailer.mcp" => Source::Mcp,
        "eu.nicemailer.web" => Source::Web,
        _ => return None,
    })
}

/// Verifies the client at the other end of `stream`: its audit token, then
/// its signature.
pub(crate) fn admit(stream: &UnixStream) -> Admission {
    match verify(stream) {
        Ok(Some(source)) => Admission::Verified(source),
        // No acceptable signature: fine only in a debug build.
        _ if cfg!(debug_assertions) => Admission::DevUnverified,
        _ => Admission::Refused,
    }
}

/// `Ok(Some(source))` when the client's signature meets the requirement and
/// its identifier is one of ours, `Ok(None)` when it has no acceptable
/// signature, `Err` when its audit token could not be read at all.
fn verify(stream: &UnixStream) -> Result<Option<Source>, ()> {
    let token = peer_audit_token(stream)?;
    // Rust note: everything below is C (CoreFoundation, Security), hence
    // `unsafe`; the compiler cannot check it. Each CoreFoundation object
    // created here is released exactly once, whatever the branch.
    unsafe {
        // The attributes {kSecGuestAttributeAudit: <the token as data>}.
        let data = CFDataCreate(std::ptr::null(), token.as_ptr().cast(), std::mem::size_of_val(&token) as isize);
        let attrs = CFDictionaryCreateMutable(
            std::ptr::null(),
            0,
            std::ptr::addr_of!(kCFTypeDictionaryKeyCallBacks).cast(),
            std::ptr::addr_of!(kCFTypeDictionaryValueCallBacks).cast(),
        );
        CFDictionarySetValue(attrs, kSecGuestAttributeAudit.cast(), data.cast());
        let mut guest: SecCodeRef = std::ptr::null();
        let rc = SecCodeCopyGuestWithAttributes(std::ptr::null(), 0, attrs, &mut guest);
        CFRelease(attrs.cast());
        CFRelease(data.cast());
        if rc != 0 || guest.is_null() {
            return Ok(None);
        }
        let result = check(guest);
        CFRelease(guest.cast());
        result
    }
}

/// `SecCodeCheckValidity` of `guest` against the requirement of this build,
/// then the source of its identifier (`kSecCodeInfoIdentifier` of its
/// signing information).
unsafe fn check(guest: SecCodeRef) -> Result<Option<Source>, ()> {
    unsafe {
        // The signing information holds the identifier of the binary.
        let mut info: CFDictionaryRef = std::ptr::null();
        let rc = SecCodeCopySigningInformation(guest, 0, &mut info);
        let ident = if rc == 0 && !info.is_null() {
            CFDictionaryGetValue(info, kSecCodeInfoIdentifier.cast()).cast::<c_void>() as CFStringRef
        } else {
            std::ptr::null()
        };
        // The value belongs to the dictionary: release the dictionary, not it.
        let identifier = if !ident.is_null() { cf_string(ident) } else { None };
        if !info.is_null() {
            CFRelease(info.cast());
        }
        let requirement = cf_string_of(&requirement());
        let mut req: SecRequirementRef = std::ptr::null();
        let rc = SecRequirementCreateWithString(requirement, &mut req);
        CFRelease(requirement.cast());
        if rc != 0 || req.is_null() {
            return Ok(None);
        }
        // kSecCSDefaultFlags = 0; errSecSuccess = 0.
        let rc = SecCodeCheckValidity(guest, 0, req);
        CFRelease(req.cast());
        if rc == 0 {
            // The signature meets the requirement; the identifier says
            // which of our binaries the client is.
            Ok(identifier.as_deref().and_then(source_of_identifier))
        } else {
            Ok(None)
        }
    }
}

/// The audit token of the peer of `stream`, recorded by the kernel at the
/// connect (`LOCAL_PEERTOKEN`).
fn peer_audit_token(stream: &UnixStream) -> Result<[u32; 8], ()> {
    // `audit_token_t` is `unsigned int val[8]` (mach/message.h).
    let mut token = [0u32; 8];
    let mut len = std::mem::size_of::<[u32; 8]>() as libc::socklen_t;
    // Rust note: `getsockopt` is a C call, hence `unsafe`. The level
    // `SYSPROTO_LOCAL` (0 on Darwin) carries the `LOCAL_*` options of a
    // Unix socket (sys/un.h).
    let rc = unsafe { libc::getsockopt(stream.as_raw_fd(), SYSPROTO_LOCAL, LOCAL_PEERTOKEN, token.as_mut_ptr().cast(), &mut len) };
    if rc == 0 { Ok(token) } else { Err(()) }
}

/// The code-signing requirement of this build: in release, an Apple-signed
/// binary of our team with one of our identifiers (`architecture.md`,
/// section 9); in debug, the identifier alone, so ad-hoc development
/// binaries pass.
fn requirement() -> String {
    let identifiers = CLIENT_IDENTIFIERS.iter().map(|i| format!("\"{i}\"")).collect::<Vec<_>>().join(", ");
    if cfg!(debug_assertions) {
        // Relaxed at compile time only, never through an environment
        // variable, which would be a back door in production.
        format!("identifier in ({identifiers})")
    } else {
        format!("anchor apple generic and certificate leaf[subject.OU] = \"{TEAM_ID}\" and identifier in ({identifiers})")
    }
}

/// A CFString as a Rust `String`, UTF-8.
unsafe fn cf_string(s: CFStringRef) -> Option<String> {
    let mut buf = [0u8; 512];
    if unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, KCF_STRING_ENCODING_UTF8) } {
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Some(String::from_utf8_lossy(&buf[..end]).into_owned())
    } else {
        None
    }
}

/// A Rust `String` as a CFString, UTF-8.
unsafe fn cf_string_of(s: &str) -> CFStringRef {
    unsafe { CFStringCreateWithCString(std::ptr::null(), s.as_ptr(), KCF_STRING_ENCODING_UTF8) }
}

// --- The C interfaces (CoreFoundation, Security) ------------------------
//
// Only what the verification needs, declared by hand rather than through a
// crate: opaque pointers, the two frameworks linked by name.

type CFStringRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFMutableDictionaryRef = *mut c_void;
type SecCodeRef = *const c_void;
type SecRequirementRef = *const c_void;
type OSStatus = i32;

/// `SYSPROTO_LOCAL` on Darwin (sys/socket.h): the level of the `LOCAL_*`
/// options of a Unix socket.
const SYSPROTO_LOCAL: libc::c_int = 0;
/// `LOCAL_PEERTOKEN` (sys/un.h): retrieve the peer's audit token.
const LOCAL_PEERTOKEN: libc::c_int = 0x006;
/// `kCFStringEncodingUTF8`.
const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

#[link(name = "CoreFoundation", kind = "framework")]
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecGuestAttributeAudit: CFStringRef;
    static kSecCodeInfoIdentifier: CFStringRef;
    static kCFTypeDictionaryKeyCallBacks: *const c_void;
    static kCFTypeDictionaryValueCallBacks: *const c_void;
    fn CFDataCreate(allocator: *const c_void, bytes: *const u8, length: isize) -> *const c_void;
    fn CFDictionaryCreateMutable(allocator: *const c_void, capacity: isize, key_callbacks: *const c_void, value_callbacks: *const c_void) -> CFMutableDictionaryRef;
    fn CFDictionarySetValue(the_dict: CFMutableDictionaryRef, key: *const c_void, value: *const c_void);
    fn CFDictionaryGetValue(the_dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
    fn CFStringCreateWithCString(alloc: *const c_void, c_str: *const u8, encoding: u32) -> CFStringRef;
    fn CFStringGetCString(the_string: CFStringRef, buffer: *mut u8, buffer_size: isize, encoding: u32) -> bool;
    fn CFRelease(cf: *const c_void);
    fn SecCodeCopyGuestWithAttributes(host: SecCodeRef, flags: u32, attributes: CFMutableDictionaryRef, guest: *mut SecCodeRef) -> OSStatus;
    fn SecCodeCopySigningInformation(code: SecCodeRef, flags: u32, info: *mut CFDictionaryRef) -> OSStatus;
    fn SecCodeCheckValidity(code: SecCodeRef, flags: u32, requirement: SecRequirementRef) -> OSStatus;
    fn SecRequirementCreateWithString(requirement: CFStringRef, out: *mut SecRequirementRef) -> OSStatus;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identifier_of_a_signed_client_gives_its_source() {
        assert_eq!(source_of_identifier("eu.nicemailer.NiceMailer"), Some(Source::App));
        assert_eq!(source_of_identifier("eu.nicemailer.cli"), Some(Source::Cli));
        assert_eq!(source_of_identifier("eu.nicemailer.tui"), Some(Source::Tui));
        assert_eq!(source_of_identifier("eu.nicemailer.mcp"), Some(Source::Mcp));
        assert_eq!(source_of_identifier("eu.nicemailer.web"), Some(Source::Web));
        assert_eq!(source_of_identifier("com.evil.trojan"), None);
    }

    #[test]
    fn the_requirement_names_our_identifiers_and_in_release_the_team() {
        let r = requirement();
        for i in CLIENT_IDENTIFIERS {
            assert!(r.contains(i), "{r}");
        }
        #[cfg(not(debug_assertions))]
        assert!(r.contains(&format!("subject.OU] = \"{TEAM_ID}\"")), "{r}");
        // The relaxed requirement of a debug build never demands the anchor.
        #[cfg(debug_assertions)]
        assert!(!r.contains("anchor apple"), "{r}");
    }

    /// The test binary is not signed with one of our identifiers: in a
    /// debug build it is admitted unverified, in a release build refused.
    #[tokio::test]
    async fn an_unsigned_client_is_refused_in_release_only() {
        let (a, _b) = UnixStream::pair().unwrap();
        match admit(&a) {
            Admission::Verified(_) => panic!("the test binary is not signed with one of our identifiers"),
            Admission::DevUnverified => assert!(cfg!(debug_assertions)),
            Admission::Refused => assert!(!cfg!(debug_assertions)),
        }
    }
}
