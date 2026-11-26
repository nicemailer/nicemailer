import AppKit
import Foundation

/// Connecting AI agents to NiceMailer, from the app, without a terminal
/// (`specifications/mcp.md`, section 3, "Installation"; `docs/mcp.md`).
///
/// The MCP server is the helper `Contents/Helpers/nicemailer-mcp` of the
/// bundle. Three gestures, in the app menu "AI Agents":
///
/// - "Connect to Claude Desktop" writes the entry `nicemailer` into Claude
///   Desktop's configuration. The helper does it
///   (`nicemailer-mcp claude-desktop install --json`): the rules of that
///   file (keep the other keys, never overwrite a file it cannot read,
///   write through a renamed temporary file) are in Rust, tested once
///   (`nm-mcp/src/install.rs`), and the app only shows the outcome;
/// - "Copy Command for Claude Code" puts `claude mcp add …` on the
///   clipboard;
/// - "Copy MCP Configuration" puts the JSON block for any other client.
///
/// At each launch the app repairs the entry if it exists and names
/// another binary, the app having been moved (`install --repair`).
///
/// In UI script mode the helper is the one `NICEMAILER_MCP` names, and the
/// configuration is a file of the scenario's data directory: the user's
/// Claude Desktop is never touched.
struct AgentSetup {
    /// The `nicemailer-mcp` the agents run.
    let helper: URL
    /// Claude Desktop's configuration; `nil`: the helper's default
    /// (`~/Library/Application Support/Claude/claude_desktop_config.json`).
    let desktopConfig: URL?

    /// The helper of the bundle (`make bundle`), else the one
    /// `NICEMAILER_MCP` names (UI tests). `nil` under `swift run`: the
    /// menu items are then disabled.
    static func find() -> AgentSetup? {
        let config = UIScript.isActive ? DataDirectory.current.appendingPathComponent("claude_desktop_config.json") : nil
        let bundled = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/nicemailer-mcp")
        if Bundle.main.bundleURL.pathExtension == "app", FileManager.default.isExecutableFile(atPath: bundled.path) {
            return AgentSetup(helper: bundled, desktopConfig: config)
        }
        if let path = DevEnvironment.value("NICEMAILER_MCP"),
           FileManager.default.isExecutableFile(atPath: path) {
            return AgentSetup(helper: URL(fileURLWithPath: path), desktopConfig: config)
        }
        return nil
    }

    // MARK: - What is copied (pure functions, tested)

    /// A path as one word of a shell command: as it is when it has only
    /// safe characters, else in single quotes (a quote inside written
    /// `'\''`).
    static func shellWord(_ s: String) -> String {
        let safe = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "/._-+:@%=,"))
        if !s.isEmpty, s.unicodeScalars.allSatisfy({ safe.contains($0) && $0.isASCII }) { return s }
        return "'" + s.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }

    /// The command that adds NiceMailer to Claude Code, for every project
    /// of the user (`--scope user`).
    static func claudeCodeCommand(helper: String) -> String {
        "claude mcp add --scope user nicemailer -- \(shellWord(helper))"
    }

    /// The block another MCP client takes (`mcp.md`, section 3): the
    /// command to run, nothing else (the server speaks on stdio).
    static func configBlock(helper: String) -> String {
        let escaped = helper.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
        return "{\n  \"nicemailer\": {\n    \"command\": \"\(escaped)\"\n  }\n}"
    }

    // MARK: - Claude Desktop

    /// What the helper did with Claude Desktop's configuration.
    enum Outcome: Equatable {
        case added, updated, unchanged, notInstalled
        /// The helper's message: a file it cannot read, no Claude Desktop.
        case failed(String)
    }

    /// The outcome from the helper's JSON line
    /// (`{"outcome": "added", "message": …}`).
    static func outcome(fromJSON data: Data) -> Outcome {
        guard let o = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            return .failed(String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines))
        }
        switch o["outcome"] as? String {
        case "added": return .added
        case "updated": return .updated
        case "unchanged": return .unchanged
        case "not_installed": return .notInstalled
        default: return .failed(o["message"] as? String ?? "")
        }
    }

    /// What the status bar says.
    static func message(_ o: Outcome) -> String {
        switch o {
        case .added, .updated: return L("NiceMailer is connected to Claude Desktop: restart Claude Desktop to use it")
        case .unchanged: return L("NiceMailer is already connected to Claude Desktop")
        case .notInstalled: return ""
        case .failed(let reason): return L("Claude Desktop could not be connected: %@", reason)
        }
    }

    /// Runs `nicemailer-mcp claude-desktop install [--repair] --json`.
    ///
    /// Swift note: `withCheckedContinuation` turns the end of the process
    /// into an `await`; the process waits on a queue of its own, never on
    /// the main thread.
    func connectClaudeDesktop(repair: Bool) async -> Outcome {
        var args = ["claude-desktop", "install", "--json"]
        if repair { args.append("--repair") }
        if let c = desktopConfig { args += ["--config", c.path] }
        let helper = self.helper
        return await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let p = Process()
                p.executableURL = helper
                p.arguments = args
                let out = Pipe()
                p.standardOutput = out
                p.standardError = FileHandle.nullDevice
                do {
                    try p.run()
                } catch {
                    continuation.resume(returning: .failed(error.localizedDescription))
                    return
                }
                let data = out.fileHandleForReading.readDataToEndOfFile()
                p.waitUntilExit()
                continuation.resume(returning: AgentSetup.outcome(fromJSON: data))
            }
        }
    }
}

extension AppModel {
    /// "Connect to Claude Desktop": the outcome in the status bar.
    func connectClaudeDesktop() async {
        guard let setup = AgentSetup.find() else { return }
        flash = AgentSetup.message(await setup.connectClaudeDesktop(repair: false))
    }

    /// At launch: the entry of Claude Desktop pointed again to this app's
    /// helper, if the app was moved. Silent unless it changed something.
    func repairClaudeDesktop() async {
        guard let setup = AgentSetup.find() else { return }
        if case .updated = await setup.connectClaudeDesktop(repair: true) {
            CoreAgent.log.notice("Claude Desktop's entry repaired")
        }
    }

    /// "Copy Command for Claude Code".
    func copyClaudeCodeCommand(to pasteboard: NSPasteboard = .general) {
        guard let setup = AgentSetup.find() else { return }
        pasteboard.clearContents()
        pasteboard.setString(AgentSetup.claudeCodeCommand(helper: setup.helper.path), forType: .string)
        flash = L("Command copied: paste it in a terminal")
    }

    /// "Copy MCP Configuration".
    func copyMCPConfiguration(to pasteboard: NSPasteboard = .general) {
        guard let setup = AgentSetup.find() else { return }
        pasteboard.clearContents()
        pasteboard.setString(AgentSetup.configBlock(helper: setup.helper.path), forType: .string)
        flash = L("Configuration copied: paste it in the settings of your AI client")
    }
}
