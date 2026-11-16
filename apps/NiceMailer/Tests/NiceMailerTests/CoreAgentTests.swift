import Testing
@testable import NiceMailer

/// The agent of the core (`CoreAgent`): reading what `launchctl print` says
/// of it, to tell a core launchd refuses to start from one that ran and
/// left (`docs/app-bundle.md`, "Signature").
struct CoreAgentTests {
    /// Abridged output of `launchctl print`, observed after a release was
    /// installed over a development build: the launch constraint recorded
    /// at the registration (category 3, Apple Development) refuses the new
    /// core (Developer ID).
    static let refused = """
    gui/502/eu.nicemailer.core = {
    	active count = 0
    	state = spawn scheduled
    	LWCR = {
    		"reqs" => {
    			"signing-identifier" => "eu.nicemailer.core"
    			"validation-category" => 3
    			"team-identifier" => "JLFYPD9J96"
    		}
    	}
    	runs = 13
    	last exit code = 78: EX_CONFIG
    	job state = spawn failed
    	properties = partial import | runatload | resolve program | needs LWCR update | has LWCR
    }
    """

    /// The spawn refused is recognised, whatever the indentation.
    @Test func aRefusedSpawnIsRecognised() {
        #expect(CoreAgent.spawnRefused(Self.refused))
        #expect(CoreAgent.spawnRefused("job state=spawn failed"))
    }

    /// A core running, or one that ran and stopped (a stop asked, an error
    /// it reported in its folder), is not a refusal: registering the agent
    /// again would not help, and is not done.
    @Test func aCoreThatRanIsNotARefusal() {
        let running = Self.refused.replacingOccurrences(of: "job state = spawn failed", with: "job state = running")
        let exited = Self.refused.replacingOccurrences(of: "job state = spawn failed", with: "job state = exited")
        #expect(!CoreAgent.spawnRefused(running))
        #expect(!CoreAgent.spawnRefused(exited))
        #expect(!CoreAgent.spawnRefused(""))
        // Only the "job state" line counts, not other lines mentioning it.
        #expect(!CoreAgent.spawnRefused("last reason = spawn failed"))
    }
}
