import Foundation
import OSLog
import Security

/// Whether the program at the other end of the app's socket is NiceMailer's
/// core, from its code signature (`architecture.md`, section 9). The core
/// verifies the app the same way (`crates/nm-signing`); this is the other
/// direction: before the app sends a password or a draft, it makes sure no
/// other program listens on the socket in place of the core — one started
/// with a `NICEMAILER_DATA_DIR` of its choosing, for instance.
///
/// The steps, as in the core: the peer's audit token
/// (`getsockopt(LOCAL_PEERTOKEN)`, never the PID, which the system may
/// give to another process), then `SecCodeCopyGuestWithAttributes` and
/// `SecCodeCheckValidity` against a requirement built from the app's own
/// signature: its team, the identifier `eu.nicemailer.core`, a Developer
/// ID when the app has one, the hardened runtime, no `get-task-allow`.
///
/// Nothing is checked in a debug build, nor when the app itself has no team
/// (an ad-hoc bundle, `swift run`): its core is then a development build
/// too, unsigned. Replacing the app's own signature is beyond what the
/// check protects against.
enum CoreSignature {
    static let log = Logger(subsystem: "eu.nicemailer.app", category: "core-signature")

    /// The identifier of the core's signature (`scripts/sign-core.sh`).
    static let coreIdentifier = "eu.nicemailer.core"

    /// `nil` when the peer of `fd` may be talked to; otherwise why not, for
    /// the log.
    static func refusal(fd: Int32) -> String? {
        guard let team = ownTeam else { return nil }
        guard let token = peerAuditToken(fd: fd) else { return "no audit token" }
        return refusal(token: token, requirement: requirement(team: team, developerID: ownIsDeveloperID))
    }

    /// The requirement the core must meet, in Apple's requirement language
    /// (`man csreq`): the same as the core's for its clients
    /// (`nm_signing::requirement`).
    static func requirement(team: String, developerID: Bool) -> String {
        var r = "anchor apple generic and certificate leaf[subject.OU] = \"\(team)\" and (identifier \"\(coreIdentifier)\")"
        if developerID { r += developerIDMarkers }
        r += " and !(entitlement[\"com.apple.security.get-task-allow\"] exists)"
        return r
    }

    /// The markers of a Developer ID signature (intermediate, leaf).
    static let developerIDMarkers =
        " and certificate 1[field.1.2.840.113635.100.6.2.6] and certificate leaf[field.1.2.840.113635.100.6.1.13]"

    /// Ten capital letters or digits: the form of a team id. Anything else
    /// is not written into a requirement.
    static func isTeamID(_ s: String) -> Bool {
        s.count == 10 && s.allSatisfy { ($0.isASCII && $0.isUppercase) || $0.isASCII && $0.isNumber }
    }

    // MARK: - The app's own signature, read once

    /// The team of the app, when the checks apply: a release build signed
    /// with a team. `nil` in a debug build or without a team.
    /// Swift note: a `static let` is computed once, lazily and thread-safely.
    static let ownTeam: String? = {
        guard !DevEnvironment.isDebugBuild, let me = selfCode() else { return nil }
        return signingInfo(me)[kSecCodeInfoTeamIdentifier as String].flatMap { $0 as? String }.flatMap { isTeamID($0) ? $0 : nil }
    }()

    /// Whether the app is signed with a Developer ID (a release).
    static let ownIsDeveloperID: Bool = {
        guard let me = selfCode(), let req = compile("anchor apple generic" + developerIDMarkers) else { return false }
        return SecCodeCheckValidity(me, [], req) == errSecSuccess
    }()

    // MARK: - Security framework calls

    /// `LOCAL_PEERTOKEN` (sys/un.h) at level `SOL_LOCAL` (0).
    private static let localPeerToken: Int32 = 0x006

    /// The audit token of the peer of the connected Unix socket `fd`.
    static func peerAuditToken(fd: Int32) -> audit_token_t? {
        var token = audit_token_t()
        var length = socklen_t(MemoryLayout<audit_token_t>.size)
        // Swift note: `&token` passes a pointer to the C call, valid during
        // the call only.
        guard getsockopt(fd, 0, localPeerToken, &token, &length) == 0,
              Int(length) == MemoryLayout<audit_token_t>.size else { return nil }
        return token
    }

    private static func refusal(token: audit_token_t, requirement text: String) -> String? {
        var token = token
        let data = Data(bytes: &token, count: MemoryLayout<audit_token_t>.size)
        let attributes = [kSecGuestAttributeAudit as String: data] as CFDictionary
        var guest: SecCode?
        guard SecCodeCopyGuestWithAttributes(nil, attributes, [], &guest) == errSecSuccess, let guest else {
            return "no code for the peer"
        }
        guard let req = compile(text) else { return "invalid requirement \(text)" }
        let status = SecCodeCheckValidity(guest, [], req)
        guard status == errSecSuccess else { return "signature refused (\(status))" }
        // The hardened runtime is a flag of the code directory, which a
        // requirement cannot express.
        let flags = (signingInfo(guest)[kSecCodeInfoFlags as String] as? NSNumber)?.uint32Value ?? 0
        guard flags & 0x0001_0000 != 0 else { return "the core lacks the hardened runtime" }
        return nil
    }

    private static func compile(_ text: String) -> SecRequirement? {
        var req: SecRequirement?
        guard SecRequirementCreateWithString(text as CFString, [], &req) == errSecSuccess else { return nil }
        return req
    }

    private static func selfCode() -> SecCode? {
        var me: SecCode?
        guard SecCodeCopySelf([], &me) == errSecSuccess else { return nil }
        return me
    }

    /// The signing information of `code`, with the signing details (the
    /// team is only there with `kSecCSSigningInformation`).
    private static func signingInfo(_ code: SecCode) -> [String: Any] {
        var staticCode: SecStaticCode?
        guard SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode else { return [:] }
        var info: CFDictionary?
        guard SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &info) == errSecSuccess,
              let dict = info as? [String: Any] else { return [:] }
        return dict
    }
}
