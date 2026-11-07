import Foundation
import Testing
@testable import NiceMailer

/// Where the app looks for the core (`DataDirectory`): the same rule as the
/// core and the CLI (`nm_api::paths::resolve`).
struct DataDirectoryTests {
    let fallback = "/Users/me/Library/Application Support/NiceMailer"

    @Test func theVariableThenTheChoiceThenTheDefault() {
        #expect(DataDirectory.resolve(env: "/tmp/test", choice: "/Users/me/Archive", defaultPath: fallback) == "/tmp/test")
        #expect(DataDirectory.resolve(env: nil, choice: "  /Users/me/Archive\n", defaultPath: fallback) == "/Users/me/Archive")
        #expect(DataDirectory.resolve(env: nil, choice: nil, defaultPath: fallback) == fallback)
    }

    @Test func aBrokenChoiceIsIgnored() {
        #expect(DataDirectory.resolve(env: nil, choice: "", defaultPath: fallback) == fallback)
        #expect(DataDirectory.resolve(env: nil, choice: "relative/dir", defaultPath: fallback) == fallback)
        #expect(DataDirectory.resolve(env: "", choice: nil, defaultPath: fallback) == fallback)
    }

    @Test func theNameShownIsTheFoldersOwn() {
        #expect(DataDirectory.displayName(URL(fileURLWithPath: "/Users/me/Library/Application Support/NiceMailer-archive")) == "NiceMailer-archive")
    }

    @Test func theTestsAreNeverAnAgent() {
        // `swift test` runs a bare executable: no agent, the core by hand.
        #expect(!CoreAgent.isBundled)
        #expect(CoreAgent.register() == .notBundled)
    }
}
