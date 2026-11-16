import Foundation
import Testing
@testable import NiceMailer

/// Connecting AI agents (`AgentSetup`, `docs/mcp.md`): what is copied, the
/// outcome of the helper, and the arguments it is run with. The helper
/// itself (the rules of Claude Desktop's file) is tested in Rust
/// (`nm-mcp/src/install.rs`, `nm-mcp/tests/binary.rs`).
struct AgentSetupTests {
    static let bundled = "/Applications/NiceMailer.app/Contents/Helpers/nicemailer-mcp"

    @Test func theCommandForClaudeCodeRunsTheHelper() {
        #expect(AgentSetup.claudeCodeCommand(helper: Self.bundled)
            == "claude mcp add --scope user nicemailer -- /Applications/NiceMailer.app/Contents/Helpers/nicemailer-mcp")
        // A path with a space or a quote is one word for the shell.
        #expect(AgentSetup.claudeCodeCommand(helper: "/Users/me/My Apps/nicemailer-mcp")
            == "claude mcp add --scope user nicemailer -- '/Users/me/My Apps/nicemailer-mcp'")
        #expect(AgentSetup.shellWord("/a/it's") == "'/a/it'\\''s'")
        #expect(AgentSetup.shellWord("/Users/me/Éric/x") == "'/Users/me/Éric/x'")
        #expect(AgentSetup.shellWord("") == "''")
    }

    @Test func theConfigurationBlockIsJSONNamingTheHelper() throws {
        for path in [Self.bundled, "/Users/me/a \"b\"\\c"] {
            let block = AgentSetup.configBlock(helper: path)
            let o = try #require(try JSONSerialization.jsonObject(with: Data(block.utf8)) as? [String: [String: String]])
            #expect(o["nicemailer"]?["command"] == path)
        }
    }

    @Test func theOutcomeOfTheHelperIsRead() {
        let json = { (s: String) in AgentSetup.outcome(fromJSON: Data(s.utf8)) }
        #expect(json(#"{"outcome":"added","message":"x"}"#) == .added)
        #expect(json(#"{"outcome":"updated"}"#) == .updated)
        #expect(json(#"{"outcome":"unchanged"}"#) == .unchanged)
        #expect(json(#"{"outcome":"not_installed"}"#) == .notInstalled)
        #expect(json(#"{"outcome":"error","message":"not valid JSON"}"#) == .failed("not valid JSON"))
        // A helper that crashed or printed something else: said as it is.
        #expect(json("boom\n") == .failed("boom"))
        // In the language of the Mac running the tests.
        #expect(AgentSetup.message(.added) == L("NiceMailer is connected to Claude Desktop: restart Claude Desktop to use it"))
        #expect(AgentSetup.message(.updated) == AgentSetup.message(.added))
        #expect(AgentSetup.message(.failed("no folder")).hasSuffix("no folder"))
        #expect(AgentSetup.message(.notInstalled).isEmpty)
    }

    /// The helper is run with the arguments of the contract between the
    /// two (`nicemailer-mcp claude-desktop install --json [--repair]
    /// [--config <file>]`): a script stands for it and says what it got.
    @Test func theHelperIsRunWithTheRightArguments() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("agent-setup-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let script = dir.appendingPathComponent("nicemailer-mcp")
        try "#!/bin/sh\nprintf '{\"outcome\":\"error\",\"message\":\"%s\"}' \"$*\"\n".write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: script.path)
        let config = dir.appendingPathComponent("claude.json")
        let setup = AgentSetup(helper: script, desktopConfig: config)
        #expect(await setup.connectClaudeDesktop(repair: false) == .failed("claude-desktop install --json --config \(config.path)"))
        #expect(await setup.connectClaudeDesktop(repair: true) == .failed("claude-desktop install --json --repair --config \(config.path)"))
        let missing = AgentSetup(helper: dir.appendingPathComponent("absent"), desktopConfig: nil)
        if case .failed = await missing.connectClaudeDesktop(repair: false) {} else { Issue.record("a missing helper must fail") }
    }

    @Test func theTestsHaveNoHelperUnlessNamed() {
        // `swift test` is no bundle, and names no helper.
        if ProcessInfo.processInfo.environment["NICEMAILER_MCP"] == nil {
            #expect(AgentSetup.find() == nil)
        }
    }
}
