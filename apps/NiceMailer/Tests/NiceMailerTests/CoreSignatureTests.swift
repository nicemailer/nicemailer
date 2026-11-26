import Foundation
import Security
import Testing
@testable import NiceMailer

/// The app's check of the core at the other end of its socket
/// (`CoreSignature`, `architecture.md`, section 9).
struct CoreSignatureTests {
    /// The requirement compiles, in both of its forms: a syntax error would
    /// refuse every core at run time.
    @Test func theRequirementCompiles() {
        for developerID in [false, true] {
            let text = CoreSignature.requirement(team: "ABCDE12345", developerID: developerID)
            var req: SecRequirement?
            #expect(SecRequirementCreateWithString(text as CFString, [], &req) == errSecSuccess, "\(text)")
            #expect(text.contains("identifier \"eu.nicemailer.core\""))
            #expect(text.contains("subject.OU] = \"ABCDE12345\""))
            #expect(text.contains("get-task-allow"))
            #expect(text.contains("6.1.13") == developerID)
        }
    }

    @Test func aTeamIDIsTenCapitalsOrDigits() {
        #expect(CoreSignature.isTeamID("ABCDE12345"))
        #expect(!CoreSignature.isTeamID("abcde12345"))
        #expect(!CoreSignature.isTeamID("ABCDE1234"))
        #expect(!CoreSignature.isTeamID("ABCD\"12345"))
    }

    /// The token of a socket pair's peer is read; the tests are a debug
    /// build, which checks nothing: any peer may be talked to.
    @Test func aDebugBuildTalksToAnyCore() {
        var fds: [Int32] = [0, 0]
        #expect(socketpair(AF_UNIX, SOCK_STREAM, 0, &fds) == 0)
        defer { close(fds[0]); close(fds[1]) }
        #expect(CoreSignature.peerAuditToken(fd: fds[0]) != nil)
        #expect(CoreSignature.peerAuditToken(fd: -1) == nil)
        #expect(CoreSignature.ownTeam == nil)
        #expect(CoreSignature.refusal(fd: fds[0]) == nil)
    }
}
