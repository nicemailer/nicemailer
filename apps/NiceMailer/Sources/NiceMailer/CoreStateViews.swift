import SwiftUI

// The two screens the window shows instead of the mail, in every theme:
// while the core is in a maintenance that ends by itself, and when it
// cannot be reached. They were part of the first interface (the former
// classic theme, removed in November 2026: `specifications/ui.md`).

/// The core is in a maintenance that ends by itself, an update or an
/// upgrade of the mailbox (`AppModel.waitOutMaintenance`): what is
/// happening, the mailbox, and a spinner; the app goes on by itself.
struct UpgradingView: View {
    let message: String

    var body: some View {
        VStack(spacing: 12) {
            ProgressView()
            Text(message).multilineTextAlignment(.center).frame(maxWidth: 520)
            Text(DataDirectory.current.path)
                .font(.system(.body, design: .monospaced))
                .textSelection(.enabled)
                .foregroundStyle(.secondary)
        }
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// Shown when the core cannot be reached: why, the mailbox looked for,
/// and the ways out (start it, approve it, another mailbox, try again).
struct ConnectionErrorView: View {
    @Environment(AppModel.self) private var model
    let message: String

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "bolt.horizontal.circle").font(.largeTitle).foregroundStyle(.secondary)
            Text(message).multilineTextAlignment(.center)
            // The mailbox the app looks for (`DataDirectory`).
            Text(DataDirectory.current.path)
                .font(.system(.body, design: .monospaced))
                .textSelection(.enabled)
                .foregroundStyle(.secondary)
            if model.agentState == .notBundled {
                // Started by hand in development (`make core`).
                Text(L("Start the core: make core"))
                    .foregroundStyle(.secondary)
            }
            HStack {
                if model.agentState == .requiresApproval {
                    Button(L("Open Login Items")) { CoreAgent.openLoginItemsSettings() }
                }
                // The way out when the core refuses this mailbox (an old
                // database, a wrong folder chosen): another one, checked
                // before the switch (`AppModel.changeMailbox`).
                Button(L("Change Mailbox…")) {
                    if let url = MailboxChooser.choose() { Task { await model.changeMailbox(to: url) } }
                }
                // A migration that failed: its details are in the core's
                // log, which the user may send (`mise-a-jour.md`, section 4).
                if model.lastMaintenance?.reason == .migrationFailed {
                    Button(L("Show the Core Log")) {
                        model.openFile(DataDirectory.current.appendingPathComponent("core.log"))
                    }
                    .uiTarget("show-core-log")
                }
                Button(L("Try Again")) {
                    Task { await model.retry() }
                }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
