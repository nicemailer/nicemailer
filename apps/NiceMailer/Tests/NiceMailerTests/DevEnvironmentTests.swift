import Testing
@testable import NiceMailer

/// The test variables drive a debug build only (`DevEnvironment`): a
/// release app launched by another program with `NICEMAILER_UI_SCRIPT`
/// must not play a script with the app's rights.
struct DevEnvironmentTests {
    let env = ["NICEMAILER_UI_SCRIPT": "/tmp/evil.uiscript", "NICEMAILER_CLI": "", "NICEMAILER_DATA_DIR": "/tmp/x"]

    @Test func aReleaseBuildIgnoresTheTestVariables() {
        #expect(DevEnvironment.value("NICEMAILER_UI_SCRIPT", in: env, debug: false) == nil)
    }

    @Test func aDebugBuildReadsThem() {
        #expect(DevEnvironment.value("NICEMAILER_UI_SCRIPT", in: env, debug: true) == "/tmp/evil.uiscript")
        // Empty or absent: none.
        #expect(DevEnvironment.value("NICEMAILER_CLI", in: env, debug: true) == nil)
        #expect(DevEnvironment.value("NICEMAILER_MCP", in: env, debug: true) == nil)
    }

    /// `swift test` builds in debug: the tests themselves run with the
    /// variables honored, as the UI scenarios do.
    @Test func theTestsAreADebugBuild() {
        #expect(DevEnvironment.isDebugBuild)
    }
}
