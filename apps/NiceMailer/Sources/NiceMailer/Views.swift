import SwiftUI

// SwiftUI notes for this file:
// - A view is a struct conforming to `View`. Its `body` describes what to
//   display for the current state; SwiftUI calls it again whenever a
//   property it read changes, and updates the screen accordingly. Views are
//   cheap descriptions, not long-lived objects.
// - `@Environment(AppModel.self)` reads the shared model injected at the
//   top of the hierarchy (`.environment(model)` in `NiceMailerApp`), so it
//   does not have to be passed down through every view.
// - `$model.something` is a *binding*: a two-way connection that lets a
//   control (list selection, text field) both read and write the property.
//   `@Bindable var model = model` is needed to create bindings to an
//   `@Observable` object obtained from the environment.
// - Modifiers (`.padding()`, `.font(...)`) each return a new view wrapping
//   the previous one, which is why their order matters.

/// Root view: a progress indicator, an error screen, or the three-column
/// mail interface, depending on the connection state.
struct ContentView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        // `Group` gathers views without adding any layout, so that the
        // modifiers below apply whichever branch is shown.
        Group {
            switch model.status {
            case .connecting:
                ProgressView(model.startupPhase.message ?? L("Connecting to the core…"))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            case .upgrading(let message):
                UpgradingView(message: message)
            case .failed(let message):
                ConnectionErrorView(message: message)
            case .connected:
                // Standard macOS three-column layout: sidebar, list, detail.
                NavigationSplitView {
                    FolderSidebar()
                        .navigationSplitViewColumnWidth(min: 170, ideal: 200)
                } content: {
                    ThreadColumn()
                        .navigationSplitViewColumnWidth(min: 300, ideal: 380)
                } detail: {
                    ThreadDetail()
                }
                .searchable(text: $model.searchText, placement: .sidebar, prompt: "Rechercher")
            }
        }
        // Single-letter shortcuts, without ⌘. Keys with ⌘ or ⌥ are left to
        // the menus. Returning `.ignored` lets the key reach other handlers;
        // a focused text field (the search) receives typing before this.
        .onKeyPress(phases: .down) { press in
            guard press.modifiers.isEmpty || press.modifiers == .shift else { return .ignored }
            return handleKey(press.characters.lowercased())
        }
        // The alert is shown while `lastError` is not nil. `isPresented`
        // needs a `Binding<Bool>`, built here by hand from the optional:
        // `get` says whether there is an error, `set(false)` clears it.
        // A mailbox of an earlier data format chosen: upgrade it or not
        // (the geek theme asks the same in its status bar, `Choice`).
        .confirmationDialog(
            model.pendingUpgrade.map {
                L("The mailbox %@ uses an older data format (%lld; this NiceMailer uses %lld)", DataDirectory.displayName($0.url), $0.from, $0.to)
            } ?? "",
            isPresented: Binding(
                get: { model.pendingUpgrade != nil },
                set: { if !$0, model.pendingUpgrade != nil { model.cancelMailboxUpgrade() } }
            )
        ) {
            Button(L("Upgrade and Use It")) { model.confirmMailboxUpgrade() }
            Button(L("Keep the Current Mailbox"), role: .cancel) { model.cancelMailboxUpgrade() }
        } message: {
            Text(L("Older versions of NiceMailer will no longer open it: copy the folder first to keep them able to."))
        }
        .alert("Erreur", isPresented: Binding(
            get: { model.lastError != nil },
            set: { if !$0 { model.lastError = nil } }
        )) {
            Button("OK") { model.lastError = nil }
        } message: {
            Text(model.lastError ?? "")
        }
    }

    /// Shortcuts: a folder's letter moves the thread there, j/k navigate,
    /// u toggles read/unread, f toggles the flag.
    ///
    /// Only the targets the core offers for the current folder are
    /// accepted, so pressing "T" in Quarantine does nothing. Note that u and
    /// f are not folder letters, so they never clash with a move.
    private func handleKey(_ key: String) -> KeyPress.Result {
        switch key {
        case "j": model.selectNext(1); return .handled
        case "k": model.selectNext(-1); return .handled
        case "u": model.toggleSeen(); return .handled
        case "f": model.toggleFlag(); return .handled
        default:
            guard let folder = model.selectedFolder, model.selectedThread != nil,
                  let target = folder.moveTargets.first(where: { Role.letter($0).lowercased() == key })
            else { return .ignored }
            model.move(to: target)
            return .handled
        }
    }
}

/// Shown when the core cannot be reached, with the command to start it.
/// The core upgrades the mailbox (`AppModel.waitForUpgrade`): what is
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
                Button(L("Try Again")) {
                    model.coreStoppedByUser = false
                    Task { await model.connect() }
                }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

// MARK: - Folders
// (`// MARK: -` comments create sections in Xcode's jump bar.)

/// Sidebar: folders grouped by category (Travail, Rangement...), and by
/// account when there are several.
struct FolderSidebar: View {
    @Environment(AppModel.self) private var model

    /// Accounts in the order of the folder list (the core sorts folders by
    /// account), each once: (id, name).
    private var accounts: [(Int, String)] {
        var seen = Set<Int>()
        return model.folders.compactMap { f in
            // `insert` returns whether the element was new; `.inserted` is
            // false for an account already listed.
            seen.insert(f.accountId).inserted ? (f.accountId, f.accountName) : nil
        }
    }

    var body: some View {
        @Bindable var model = model
        let accounts = self.accounts
        // Binding the selection to `selectedFolderId`: clicking a row writes
        // the row's `.tag` there, and the model's `didSet` loads the folder.
        List(selection: $model.selectedFolderId) {
            ForEach(accounts, id: \.0) { accountId, accountName in
                // `id: \.0` tells `ForEach` to identify each tuple by its
                // first element (the category key). `\.0` is a *key path*.
                ForEach(Role.categoryTitles, id: \.0) { category, title in
                    let items = model.folders.filter { $0.accountId == accountId && $0.category == category }
                    if !items.isEmpty {
                        // With a single account, its name would only add noise.
                        Section(accounts.count > 1 ? "\(accountName) · \(title)" : title) {
                            ForEach(items) { folder in
                                FolderRow(folder: folder).tag(folder.id)
                            }
                        }
                    }
                }
            }
        }
        .listStyle(.sidebar)
    }
}

/// One folder: shortcut letter, name, unread count.
struct FolderRow: View {
    let folder: FolderView

    var body: some View {
        HStack {
            Text(folder.letter ?? " ")
                .font(.system(.caption, design: .monospaced).bold())
                .foregroundStyle(.secondary)
                .frame(width: 14)
            Text(folder.name)
            Spacer()
            if folder.unreadCount > 0 {
                Text("\(folder.unreadCount)").font(.caption).foregroundStyle(.secondary).monospacedDigit()
            }
        }
    }
}

// MARK: - Threads

/// Middle column: the folder's threads, or the search results while the
/// search field is not empty.
struct ThreadColumn: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if model.searchText.isEmpty {
            ThreadList()
                .navigationTitle(model.selectedFolder?.name ?? "")
                .navigationSubtitle(model.selectedFolder.map { "\($0.threadCount) conversations" } ?? "")
        } else {
            SearchResults()
                .navigationTitle("Recherche")
        }
    }
}

/// The folder's threads. `List` only builds the rows that are visible, so a
/// long list stays fast.
struct ThreadList: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        List(selection: $model.selectedThreadId) {
            ForEach(model.threads) { thread in
                ThreadRow(thread: thread, folderRole: model.selectedFolder?.role)
                    .tag(thread.id)
                    // `.task` runs when the row appears on screen. For the
                    // last row, this loads the next page: infinite scrolling.
                    .task { await model.loadMoreIfNeeded(after: thread) }
            }
        }
        .listStyle(.inset)
        .overlay {
            if model.threads.isEmpty {
                Text("Aucune conversation").foregroundStyle(.secondary)
            }
        }
    }
}

/// One thread in the list: unread dot, participants, message count, icons
/// and date on the first line; return badge and subject on the second;
/// snippet below.
struct ThreadRow: View {
    let thread: ThreadSummary
    let folderRole: String?

    var body: some View {
        let unread = thread.unreadCount > 0
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 4) {
                // A transparent dot when read keeps the text aligned.
                Circle().fill(unread ? Color.accentColor : .clear).frame(width: 7, height: 7)
                Text(thread.participants.joined(separator: ", "))
                    .fontWeight(unread ? .semibold : .regular)
                    .lineLimit(1)
                if thread.messageCount > 1 {
                    Text("\(thread.messageCount)").font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                if thread.hasAttachments { Image(systemName: "paperclip").foregroundStyle(.secondary) }
                if thread.isFlagged { Image(systemName: "flag.fill").foregroundStyle(.orange) }
                Text(Format.listDate(thread.dateLastReceived))
                    .font(.caption).foregroundStyle(.secondary).monospacedDigit()
            }
            HStack(spacing: 6) {
                // "↩ Todo" badge: the thread came back from Todo or Waiting.
                if let from = thread.returnedFrom {
                    Text("↩ \(Role.name(from))")
                        .font(.caption.bold())
                        .padding(.horizontal, 5).padding(.vertical, 1)
                        .background(.yellow.opacity(0.3), in: RoundedRectangle(cornerRadius: 4))
                }
                Text(thread.subject).lineLimit(1)
            }
            // 11 points = dot width + spacing, to align under the names.
            .padding(.leading, 11)
            if let snippet = thread.snippet {
                Text(snippet)
                    .font(.callout).foregroundStyle(.secondary)
                    .lineLimit(2)
                    .padding(.leading, 11)
            }
        }
        .padding(.vertical, 3)
    }
}

/// Search results: one row per matching message, with the matched terms in
/// bold. Clicking a result opens its thread in its folder.
struct SearchResults: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List(model.searchResults) { hit in
            // A `.plain` button: the whole row is clickable, without the look
            // of a button.
            Button { model.openSearchHit(hit) } label: {
                VStack(alignment: .leading, spacing: 2) {
                    HStack {
                        Text(hit.fromName ?? hit.fromAddr ?? "").bold().lineLimit(1)
                        Spacer()
                        Text(Role.name(hit.folderRole)).font(.caption).foregroundStyle(.secondary)
                        Text(Format.listDate(hit.dateReceived)).font(.caption).foregroundStyle(.secondary)
                    }
                    Text(hit.subject).lineLimit(1)
                    Text(highlighted(hit.highlight)).font(.callout).foregroundStyle(.secondary).lineLimit(2)
                }
            }
            .buttonStyle(.plain)
        }
        .overlay {
            if model.searchResults.isEmpty { Text("Aucun résultat").foregroundStyle(.secondary) }
        }
    }

    /// Turns the core's excerpt into styled text: the core wraps matched
    /// terms in `[` and `]`, which become bold, primary-colored runs.
    ///
    /// The string is walked character by character; `current` accumulates
    /// the text of the current run, and `flush` appends it with the right
    /// style when a bracket is met. `flush` is a nested function: it can read
    /// and modify the local variables around it.
    private func highlighted(_ s: String) -> AttributedString {
        var result = AttributedString()
        var bold = false
        var current = ""
        func flush() {
            var part = AttributedString(current)
            if bold { part.font = .callout.bold(); part.foregroundColor = .primary }
            result += part
            current = ""
        }
        for c in s {
            if c == "[" { flush(); bold = true } else if c == "]" { flush(); bold = false } else { current.append(c) }
        }
        flush()
        return result
    }
}

// MARK: - Reading

/// Reading pane: every message of the thread, oldest first.
struct ThreadDetail: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        // Only show the loaded thread if it is still the selected one: right
        // after a new selection, the previous thread stays in `openThread`
        // until the new one is loaded.
        if let view = model.openThread, view.id == model.selectedThreadId {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text(view.subject).font(.title2.bold()).textSelection(.enabled)
                    ForEach(view.messages) { message in
                        MessageCard(message: message)
                    }
                }
                .padding(20)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .toolbar { ThreadToolbar() }
        } else {
            Text("Aucune conversation sélectionnée")
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

/// Toolbar buttons for the selected thread: read/unread, flag, move menu.
/// Each one mirrors a keyboard shortcut, shown as a tooltip (`.help`).
struct ThreadToolbar: ToolbarContent {
    @Environment(AppModel.self) private var model

    var body: some ToolbarContent {
        ToolbarItemGroup {
            if let thread = model.selectedThread {
                Button { model.toggleSeen() } label: {
                    Label(thread.unreadCount > 0 ? "Marquer comme lu" : "Marquer comme non lu",
                          systemImage: thread.unreadCount > 0 ? "envelope.open" : "envelope.badge")
                }
                .help("U")
                Button { model.toggleFlag() } label: {
                    Label("Drapeau", systemImage: thread.isFlagged ? "flag.slash" : "flag")
                }
                .help("F")
                Menu {
                    ForEach(model.selectedFolder?.moveTargets ?? [], id: \.self) { target in
                        Button("\(Role.letter(target))   \(Role.name(target))") { model.move(to: target) }
                    }
                } label: {
                    Label("Déplacer", systemImage: "folder")
                }
                .disabled(model.selectedFolder?.moveTargets.isEmpty ?? true)
            }
        }
    }
}

/// One message: header (sender, recipients, date, folder, warnings) and
/// plain text body. The user's own messages (in Sent) are slightly faded so
/// the two sides of the conversation can be told apart.
struct MessageCard: View {
    let message: MessageView

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(message.fromName ?? message.fromAddr ?? "?").bold()
                        if let addr = message.fromAddr, message.fromName != nil {
                            Text(addr).foregroundStyle(.secondary)
                        }
                        if message.authResult == "fail" {
                            Label("Authentification échouée", systemImage: "exclamationmark.shield")
                                .font(.caption).foregroundStyle(.red)
                        }
                    }
                    if !message.to.isEmpty {
                        Text("À : " + message.to.map(\.display).joined(separator: ", "))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    if !message.cc.isEmpty {
                        Text("Cc : " + message.cc.map(\.display).joined(separator: ", "))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
                Spacer()
                VStack(alignment: .trailing, spacing: 2) {
                    Text(Format.longDate(message.dateReceived)).font(.caption).foregroundStyle(.secondary)
                    HStack(spacing: 4) {
                        if message.hasAttachments { Image(systemName: "paperclip") }
                        if message.isFlagged { Image(systemName: "flag.fill").foregroundStyle(.orange) }
                        Text(Role.name(message.folderRole))
                            .font(.caption)
                            .padding(.horizontal, 5).padding(.vertical, 1)
                            .background(.quaternary, in: RoundedRectangle(cornerRadius: 4))
                    }
                    .foregroundStyle(.secondary)
                }
            }
            Divider()
            Text(message.bodyText ?? "(corps non téléchargé)")
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(14)
        .background(.background, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(.quaternary))
        .opacity(message.folderRole == "sent" ? 0.85 : 1)
    }
}
