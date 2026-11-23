import AppKit
import SwiftUI
import WebKit

// Views of the geek theme (`ui.md`): an IDE-like window without rounded
// zones, scroll bars or animations; a compact status bar; Helix-style
// menus and pickers.

/// Root of the geek theme. Installs the keyboard monitor and lays out the
/// columns, the status bar and the overlays.
struct RootView: View {
    @Environment(AppModel.self) private var model
    @State private var controller: InterfaceController?
    /// The AppKit event monitor, removed when the view goes away.
    @State private var monitor: Any?
    /// The window of this view: the monitor takes only its keys.
    @State private var mainWindow = MainWindowReference()

    var body: some View {
        Group {
            if let controller {
                WindowContent(controller: controller)
            } else {
                Palette.background
            }
        }
        .background(MainWindowReader(reference: mainWindow))
        .onAppear {
            let c = InterfaceController(model: model)
            controller = c
            // The editor of the replies' text (`EditorWebView.swift`), made
            // by the model once the first list is shown (`ensureEditor`).
            // The model and the controller live as long as the app.
            let m = model
            model.makeEditor = { [weak c] in
                let bridge = EditorBridge.shared
                bridge.onChanged = { content in m.editorChanged(content) }
                bridge.onState = { state in c?.editorStateChanged(state) }
                bridge.onLeave = { c?.leaveText() }
                bridge.onFiles = { urls in Task { await m.attach(urls) } }
                return bridge
            }
            if model.startupPhase == .ready { model.ensureEditor() }
            // Swift note: the closure receives every key-down event of the
            // app, in all its windows, before the views do. Returning nil
            // swallows the event; returning it lets AppKit go on (menus,
            // ⌘ shortcuts, the fields of the other windows: `KeyRouting`).
            let window = mainWindow
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
                c.route(event, mainWindow: window.window)
            }
            // Automated UI test, when `NICEMAILER_UI_SCRIPT` is set.
            UIScript.start(model: model, controller: c)
        }
        .onDisappear {
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
        }
    }
}

/// The window content, once the controller exists.
private struct WindowContent: View {
    @Environment(AppModel.self) private var model
    let controller: InterfaceController

    var body: some View {
        // Rebuilt from scratch when a text size or the theme changes
        // (`.id`): every size and width derives from the sizes, every color
        // from the theme, and rebuilding is more reliable than counting on
        // lazy rows to notice the change.
        let textSize = "\(TextSize.shared.size)-\(TextSize.shared.statusBarSize)-\(model.theme.revision)"
        VStack(spacing: 0) {
            // A toolbar is always at the top (`window.bar_type`); the status
            // bar where `v` put it.
            if Look.toolbar || controller.statusPosition == .top { StatusBarView(controller: controller) }
            ZStack(alignment: .bottomTrailing) {
                content
                clickCatcher
                overlay
            }
            if !Look.toolbar, controller.statusPosition == .bottom { StatusBarView(controller: controller) }
        }
        .background(Palette.background)
        .foregroundStyle(Palette.text)
        .font(.interface())
        // `ui.md`: no millisecond lost in geek. Turning animations off for
        // the whole tree also removes SwiftUI's default fades. A theme
        // that animates (`[motion]`) keeps its own, given where they apply.
        .transaction { t in
            if !Look.animates { t.animation = nil; t.disablesAnimations = true }
        }
        .animation(Look.sidebarAnimation, value: controller.columns)
        // The layout of a theme arrived from the core (`window.default_columns`).
        .onChange(of: model.theme.revision, initial: true) { _, _ in controller.applyThemeLayout() }
        .animation(Look.popupAnimation, value: controller.overlay)
        .id(textSize)
    }

    @ViewBuilder private var content: some View {
        if case .maintenance(let text, _) = model.status {
            // NiceMailer updated, or the mailbox upgraded: say so, the app
            // waits.
            UpgradingView(message: text).font(.interface(12))
        } else if case .failed(let message) = model.status, !UIScript.isActive {
            // No core: what to do, and the button that allows it.
            ConnectionErrorView(message: message).font(.interface(12))
        } else {
            screenContent
        }
    }

    @ViewBuilder private var screenContent: some View {
        switch controller.screen {
        case .mail:
            MailColumns(controller: controller)
        case .contacts:
            ContactsScreen(controller: controller)
        case .history:
            HistoryScreen(controller: controller)
        case .tags:
            NamesScreen(
                controller: controller, title: L("Tags"),
                rows: controller.screenTags.map { ($0.name, L("%lld mails", $0.messageCount)) })
        case .groups:
            NamesScreen(
                controller: controller, title: L("Groups"),
                rows: controller.groups.map { ($0.name, L("%lld members", $0.memberCount)) })
        }
    }

    /// Under an open menu, picker or help, a transparent layer over the
    /// whole content: a click outside the panel lands on it and closes the
    /// panel (`clickOutsideOverlay`), instead of reaching the list or the
    /// reader under it. The status bar is not covered: a click on one of
    /// its blocks opens that block's menu or picker directly.
    @ViewBuilder private var clickCatcher: some View {
        if controller.closesOnClickOutside {
            Color.clear
                .contentShape(Rectangle())
                .onTapGesture { controller.clickOutsideOverlay() }
                // For the UI scripts: a spot in the top-right corner,
                // which no panel covers (pickers and help are centered,
                // with a margin; menus drop from the status bar's left part).
                .overlay(alignment: .topTrailing) {
                    Color.clear.frame(width: 16, height: 16).padding(8).uiTarget("outside-panel")
                }
        }
    }

    @ViewBuilder private var overlay: some View {
        // A question (a suspect move to confirm, a decision to choose)
        // comes before any other panel, in the bottom-right corner —
        // except the one the screen draws where its answer belongs
        // ("Send from", in place of the line "From" of the reply).
        if let choice = controller.activeChoice, !drawnInPlace(choice) {
            ChoicePanel(controller: controller, choice: choice)
                .padding(12)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
        } else if let run = model.aiRun {
            // The result of a prompt takes the place of the palette it was
            // chosen in, the same box at the same place, for a reply as for
            // a mail: where the eyes already are (`ia.md`, section 11).
            // Its top is the palette's top (the picker is centred in the
            // window, at a fixed height), and it grows downward as the
            // answer streams in, without moving what is already read.
            GeometryReader { geo in
                let scale = TextSize.shared.scale
                let top = max(0, (geo.size.height - PickerPopup.height(.ai, scale: scale)) / 2)
                AIPanel(run: run, maxTextHeight: geo.size.height - top - 120 * scale)
                    .frame(width: PickerPopup.width(.ai, available: geo.size.width, scale: scale))
                    .popupChrome()
                    .uiTarget("ai-panel-box")
                    .padding(.top, top)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            }
        } else {
            menusAndPickers
        }
    }

    /// Choices a screen draws itself, at the place they are about: in the
    /// headers of the reply being written, ⌥⌘F replaces the line "From"
    /// and ⇧⌘↩ the line "After sending: …" (`reponse.md`, sections 4 ter
    /// and 8), so the corner leaves them alone.
    private func drawnInPlace(_ choice: InterfaceController.Choice) -> Bool {
        (choice == .sendFrom || choice == .sendThen) && model.composer != nil
    }

    @ViewBuilder private var menusAndPickers: some View {
        switch controller.overlay {
        case .menu(let prefix):
            let menu = KeyMenu(title: menuTitle(prefix),
                               entries: controller.menuEntries(prefix).map {
                                   .init(key: $0.key, label: $0.label, muted: $0.disabled, gapBefore: $0.gapBefore, detail: $0.detail,
                                         detailMuted: $0.detailStyle == .info, virtual: $0.virtual)
                               },
                               highlighted: controller.menuIndex,
                               onSelect: { controller.chooseMenuEntry($0) })
            if controller.statusPosition != .hidden, let x = controller.menuBlockX[prefix] {
                // Each menu drops down from its block of the status bar:
                // under it when the bar is at the top, above it when the
                // bar is at the bottom (`ui.md`).
                //
                // A block near the right end (Save in the toolbar of
                // regular) would push the menu out of the window: its right
                // part cut, and the layer of the overlay widened past the
                // window, which moved the blocks, which moved the menu again
                // (a layout loop, and the columns of the list shifted). The
                // room before the menu is a spacer that gets at most `x - 6`
                // and never more than what the menu and a margin leave: the
                // menu is pushed back to the left just enough to stay whole.
                //
                // Swift note: `layoutPriority(1)` makes the HStack serve the
                // spacer first, once the menu (fixed size) has its width;
                // the last spacer takes what is left.
                HStack(spacing: 0) {
                    Spacer(minLength: 0).frame(maxWidth: max(0, x - 6)).layoutPriority(1)
                    menu
                    // At least a margin before the window's edge, as a menu
                    // of macOS keeps.
                    Spacer(minLength: 8)
                }
                .padding(.vertical, 2)
                .frame(maxWidth: .infinity, maxHeight: .infinity,
                       alignment: controller.statusPosition == .bottom ? .bottom : .top)
            } else {
                // No block to drop from (status bar hidden, or another
                // screen): bottom-right corner, as in Helix.
                menu.padding(12)
            }
        case .help:
            HelpPanel(groups: Help.groups(controller.keymap, severalAccounts: controller.hasSeveralAccounts))
        case .picker(let kind):
            // Swift note: a `GeometryReader` gives its content the size it
            // is offered, here the window's content: the links panel takes
            // as much of its width as is useful.
            GeometryReader { geometry in
                PickerPopup(controller: controller, kind: kind,
                            width: PickerPopup.width(kind, available: geometry.size.width, scale: TextSize.shared.scale),
                            available: geometry.size.height)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        case .none, .contactFilter, .choice:
            EmptyView()
        }
    }

    private func menuTitle(_ p: InterfaceController.Prefix) -> String { p.title }
}

extension InterfaceController.Prefix {
    /// The title of its menu, and the name of its block in the status bar
    /// when the theme names the blocks (`window.block_labels`).
    var title: String {
        switch self {
        case .move: L("Move")
        case .goto: L("Go to")
        case .save: L("Save")
        case .view: L("Display")
        // A menu of the user (`[menu.<name>]`): its name.
        case .user(let name): name
        }
    }
}

// MARK: - Clicks

/// Modifier held during a click.
enum ClickModifiers {
    case none, command, shift
}

extension View {
    /// A click that tells which modifier was held.
    ///
    /// Why not `onTapGesture` and the keyboard state: SwiftUI runs the tap
    /// action after the click was processed, when `NSApp.currentEvent` no
    /// longer describes it. Taps filtered by modifier (`.modifiers(.command)`)
    /// read the modifiers of the click itself. `exclusively(before:)` tries
    /// them in order, so a ⌘-click is not also seen as a plain click.
    func onClick(_ action: @escaping (ClickModifiers) -> Void) -> some View {
        gesture(
            TapGesture().modifiers(.command).onEnded { action(.command) }
                .exclusively(before: TapGesture().modifiers(.shift).onEnded { action(.shift) })
                .exclusively(before: TapGesture().onEnded { action(.none) })
        )
    }
}

// MARK: - Status bar

/// Compact status bar (`ui.md`). Each element opens with one key and a
/// click: the account with `@`, the folder with `;`, the menus `v` `m` `s`,
/// and, at the right end, the list of keys with `?`.
struct StatusBarView: View {
    @Environment(AppModel.self) private var model
    let controller: InterfaceController

    var body: some View {
        // regular: a macOS toolbar, whose buttons are the same blocks
        // (`window.bar_type = "toolbar"`, `ToolbarView`); geek and clean:
        // the status bar.
        if Look.toolbar {
            ToolbarView(controller: controller, messages: messageLine)
        } else {
            statusBar
        }
    }

    /// What the app says now, on the right of the status bar or under the
    /// title of the toolbar: an error, the result of an action, a file
    /// refused, the start, the AI at work. One line, the most urgent.
    @ViewBuilder var messageLine: some View {
        if let error = model.lastError {
            Text(error).foregroundStyle(Palette.error).lineLimit(1)
        } else if let flash = model.flash {
            Text(flash).foregroundStyle(Palette.muted).lineLimit(1).truncationMode(.middle)
        } else if case .failed(let message) = model.status {
            Text(message).foregroundStyle(Palette.error).lineLimit(1)
        } else if case .maintenance(_, let short) = model.status {
            Text(short).foregroundStyle(Palette.muted).lineLimit(1)
        } else if let problem = model.keymapProblem {
            // `keymap.toml` refused: the previous keys stay, and this
            // stays until the file is fixed (`raccourcis.md`, 3).
            Text(L("keymap.toml refused, previous keys kept: %@", problem.text))
                .foregroundStyle(Palette.error).lineLimit(1).truncationMode(.middle)
        } else if let problem = model.themeProblem {
            // A file of the theme refused: its last valid version
            // stays, and this stays until the file is fixed
            // (`themes.md`, section 3).
            Text(L("Theme file refused, last valid version kept: %@", problem.text))
                .foregroundStyle(Palette.error).lineLimit(1).truncationMode(.middle)
        } else if let starting = model.startupPhase.message {
            Text(starting).foregroundStyle(Palette.muted).lineLimit(1)
        } else if let sort = model.aiSort {
            Text(AI.sortLine(sort)).foregroundStyle(Palette.accentText).lineLimit(1)
        } else if controller.screen == .mail, model.composer == nil, let s = model.selectedSuggestion {
            // Why the AI suggests a target for the mail under the cursor.
            Text(AI.suggestionLine(s, keymap: model.keymap, names: model.folderNames)).foregroundStyle(Palette.accentText).lineLimit(1).truncationMode(.tail)
        }
    }

    /// Whether the list shows mail (a folder, or a tag, group or contact
    /// list), where the attributes and the menus of the mail apply.
    private var listsMail: Bool {
        controller.screen == .mail && (model.listFilter != nil || !model.listedFolderIds.isEmpty)
    }

    /// Whether the block `name` has something to show now: no account
    /// block with a single account, no attributes off a list of mail, no
    /// `m` and `s` in Quarantine (its targets are at the right end).
    private func draws(_ name: String) -> Bool {
        switch name {
        case "folder", "help": true
        case "account": listsMail && model.listFilter == nil && controller.hasSeveralAccounts
        case "attributes": listsMail
        case "view", "ai": controller.screen == .mail
        case "move", "save": controller.screen == .mail && controller.directTargets.isEmpty
        default: false
        }
    }

    private var statusBar: some View {
        // The blocks in the order of the theme (`window.blocks`), the gaps
        // only between two blocks shown (`StatusLayout`).
        let items = StatusLayout.items(Look.theme.window.blocks, draws: draws)
        // Pills carry their own room: closer together (`window.block_style`).
        return HStack(spacing: Self.spacing) {
            ForEach(Array(items.enumerated()), id: \.offset) { _, item in
                statusItem(item)
            }
        }
        .font(.interfaceBar(Look.pillBlocks ? 12 : 11))
        .padding(.horizontal, Look.pillBlocks ? barScaled(10) : 6)
        // The window's buttons over its left end (`window.title_bar =
        // "merged"`): room for them, and the height of a title bar, so
        // that they sit in its middle.
        .padding(.leading, model.titleBarMode == .merged ? Look.mergedButtonsWidth : 0)
        // Its height (`window.bar_height`: 20 pt in geek).
        .frame(height: model.titleBarMode == .merged ? Look.mergedBarHeight : Look.barHeight)
        // Its empty parts move the window, as a title bar does: the title
        // bar is hidden by default (`WindowChrome`).
        .background(ZStack { Palette.statusBar; WindowDragArea(onClick: controller.clickBarBackground) })
        // A line between the bar and the mails (`window.bar_line`), on the
        // side of the mails.
        .overlay(alignment: controller.statusPosition == .bottom ? .top : .bottom) {
            if Look.barLine { Palette.separator.frame(height: 1) }
        }
    }

    /// The space between two elements of the bar.
    private static var spacing: CGFloat { Look.pillBlocks ? 2 : 10 }

    /// One element of the status bar.
    @ViewBuilder
    private func statusItem(_ item: String) -> some View {
        switch item {
        case StatusLayout.gap:
            // A wide gap between two groups: with the spacing of the bar on
            // each side, 28 points apart at a 12 pt text.
            Color.clear.frame(width: max(0, barScaled(28) - Self.spacing), height: 1)
        case StatusLayout.spring:
            // The chips of the screens, the room left, then what the app
            // says, on the right.
            screenChips
            // Empty: a click there reaches the drag area under the bar
            // (`clickBarBackground`). Its place, for the UI scripts.
            Spacer(minLength: 8).uiTarget("status-empty", barType: "status_bar")
            messageLine
            rightEnd
        case "account":
            // The account ("all" for the unified view), with its icon and
            // the key that opens its picker; a click opens it too.
            StatusBlock(icon: "at", text: model.scopeName, key: controller.keyLabel(.pickerAccounts),
                        strong: true, active: controller.overlay == .picker(.accounts))
                .onTapGesture { controller.openPickerFromStatusBar(.accounts) }
                .uiTarget("status-account", barType: "status_bar")
        case "folder":
            if listsMail, model.listFilter == nil {
                StatusBlock(icon: InterfaceController.Prefix.goto.icon, text: model.currentFolderName,
                            key: controller.menuKeyLabel(.goto), strong: true, active: controller.overlay == .menu(.goto))
                    .menuAnchor(.goto, controller)
                    .onTapGesture { controller.openMenuFromStatusBar(.goto) }
                    .uiTarget("status-folder", barType: "status_bar")
            } else {
                // A tag, group or contact list ("tag: Clients"), or a screen.
                Text(controller.locationLabel)
                    .lineLimit(1)
            }
        case "attributes":
            // Attributes narrowing the list, also in a tag, group or
            // contact list ("unread mails of the group"): the magnifier
            // opens the selector (`F`); each active attribute is a chip,
            // removed by a click.
            StatusBlock(icon: "magnifyingglass", text: "", key: controller.keyLabel(.pickerAttributes), label: L("Attributes"),
                        active: controller.overlay == .picker(.attributes))
                .onTapGesture { controller.openPickerFromStatusBar(.attributes) }
                .uiTarget("status-attributes", barType: "status_bar")
            ForEach(model.attributes.chips) { chip in
                Text(chip.label + " ×")
                    .lineLimit(1)
                    .foregroundStyle(Palette.accentText)
                    .padding(.horizontal, barScaled(4))
                    .overlay(Rectangle().stroke(Palette.accent.opacity(0.5), lineWidth: 1))
                    .contentShape(Rectangle())
                    .onTapGesture { model.attributes.remove(chip.id) }
                    .uiTarget("chip-\(chip.id)")
            }
        case "view", "move", "save":
            // A menu of a prefix, an icon and the key; the menu drops down
            // from the block.
            let p = InterfaceController.Prefix(name: item)
            StatusBlock(icon: p.icon, text: "", key: controller.menuKeyLabel(p), label: p.title,
                        active: controller.overlay == .menu(p))
                .menuAnchor(p, controller)
                .onTapGesture { controller.openMenuFromStatusBar(p) }
                .uiTarget("status-\(p.uiID)", barType: "status_bar")
        case "ai":
            // The AI, with the actions on the current mail: its palette
            // works on the mail (or the reply) as `m` and `s` do.
            StatusBlock(icon: "sparkles", text: "",
                        key: controller.aiKeyLabel(composing: model.composer != nil), label: L("AI"),
                        active: controller.overlay == .picker(.ai))
                .onTapGesture { controller.openAIFromStatusBar() }
                .help(L("AI assistance"))
                .accessibilityLabel(L("AI assistance"))
                .uiTarget("status-ai", barType: "status_bar")
        case "help":
            // At the right end on every screen, a reminder that `?` exists,
            // for whoever has not learned it yet; a click opens it too.
            StatusBlock(icon: "questionmark.circle", text: "", key: controller.keyLabel(.helpShow),
                        active: controller.overlay == .help)
                .onTapGesture { controller.toggleHelpFromStatusBar() }
                .help(L("Keys"))
                .uiTarget("status-help", barType: "status_bar")
        default:
            EmptyView()
        }
    }

    /// The filters of the contacts and history screens, and the contacts
    /// filter being typed: before the room left, after the blocks.
    @ViewBuilder
    private var screenChips: some View {
        if let query = controller.contactsFilterChip {
            // The contacts filter, kept while browsing: a chip, removed by
            // a click (`ui.md`, "Écran contacts").
            Text("/" + query + " ×")
                .lineLimit(1)
                .truncationMode(.middle)
                .foregroundStyle(Palette.accentText)
                .padding(.horizontal, barScaled(4))
                .overlay(Rectangle().stroke(Palette.accent.opacity(0.5), lineWidth: 1))
                .contentShape(Rectangle())
                .onTapGesture { controller.clearContactsFilter() }
                .help(L("Contacts filtered on “%@”: click to show them all", query))
                .accessibilityLabel(L("Contacts filtered on “%@”: click to show them all", query))
                .uiTarget("chip-contacts")
        }
        if let who = controller.historySourceChip {
            // The History screen filtered on who did it: a chip, removed
            // by a click (`ui.md`, "Écran historique").
            Text(verbatim: who + " ×")
                .lineLimit(1)
                .foregroundStyle(Palette.accentText)
                .padding(.horizontal, barScaled(4))
                .overlay(Rectangle().stroke(Palette.accent.opacity(0.5), lineWidth: 1))
                .contentShape(Rectangle())
                .onTapGesture { controller.filterHistory(.everyone) }
                .help(L("History filtered on %@: click to show every action", who))
                .accessibilityLabel(L("History filtered on %@: click to show every action", who))
                .uiTarget("chip-history")
        }
        if case .contactFilter = controller.overlay {
            HStack(spacing: 0) {
                Text("/")
                CaretText(line: controller.contactsLine)
            }
            .foregroundStyle(Palette.accentText)
        }
    }

    /// The right end, after the message: the synchronization in progress,
    /// the marks, the sort, the move targets of Quarantine. They are not
    /// blocks: a theme cannot hide them (`themes.md`, section 6.1).
    @ViewBuilder
    private var rightEnd: some View {
        if let sync = model.syncLine {
            // The first synchronization of an account, never cut: the app
            // must not look frozen (`ui.md`, "Avancement").
            HStack(spacing: barScaled(4)) {
                Image(systemName: "arrow.down.circle")
                    .accessibilityHidden(true)
                Text(sync)
            }
            .foregroundStyle(Palette.muted)
            .lineLimit(1)
            .fixedSize(horizontal: true, vertical: false)
            .uiTarget("status-sync")
        }
        if !model.marked.isEmpty {
            Text(L("%lld marked", model.marked.count)).foregroundStyle(Palette.markedBar)
        }
        if model.sort == "modified" { Text(L("by modification")).foregroundStyle(Palette.muted) }
        quarantineTargets
    }
}

/// The order of the blocks of the status bar (`window.blocks`,
/// `themes.md`, section 6.1), as a pure function, tested alone.
enum StatusLayout {
    /// A wide gap between two groups of blocks.
    static let gap = "|"
    /// The room left: what follows goes to the right end, after the
    /// messages of the app.
    static let spring = "->"

    /// The elements to draw: the blocks of `blocks` that `draws` says have
    /// something to show, in that order; a gap only between two blocks
    /// drawn, never next to the spring; the spring once, at the end when
    /// the theme leaves it out (the messages of the app always show).
    static func items(_ blocks: [String], draws: (String) -> Bool) -> [String] {
        var out: [String] = []
        var pendingGap = false
        var hasSpring = false
        for b in blocks {
            switch b {
            case gap:
                pendingGap = true
            case spring:
                guard !hasSpring else { continue }
                hasSpring = true
                pendingGap = false
                out.append(b)
            default:
                guard draws(b) else { continue }
                if pendingGap, let last = out.last, last != spring { out.append(gap) }
                pendingGap = false
                out.append(b)
            }
        }
        if !hasSpring { out.append(spring) }
        return out
    }
}

extension StatusBarView {
    /// In Quarantine, the move targets with their key (`directTargets`),
    /// aligned at the right end of the bar, just before `?`: the move
    /// arrow once, then "Inbox i  Newsletter n …". On the right they stay
    /// in the same place whatever the account, the folder name and the
    /// chips on the left, so the eye finds them without searching. The
    /// messages (result of a move) come before them, on their left.
    /// Empty elsewhere.
    @ViewBuilder
    fileprivate var quarantineTargets: some View {
        let targets = controller.directTargets
        if !targets.isEmpty {
            HStack(spacing: barScaled(8)) {
                Image(systemName: InterfaceController.Prefix.move.icon)
                    .font(.system(size: barScaled(10), weight: .semibold))
                    .foregroundStyle(Palette.muted)
                    .accessibilityLabel(L("Move"))
                ForEach(targets, id: \.self) { role in
                    StatusBlock(icon: nil, text: model.folderNames.name(role),
                                key: KeyAction(rawValue: "move." + role).map(controller.keyLabel) ?? "")
                        .onTapGesture { controller.moveFromStatusBar(role) }
                        .uiTarget("status-target-\(role)")
                }
            }
            // A wide gap on each side: the targets are a group of their
            // own, apart from the messages and from `?`.
            .padding(.horizontal, barScaled(18))
            // Never squeezed: a long message on their left is cut instead.
            .fixedSize(horizontal: true, vertical: false)
        }
    }
}

/// The command of the window in regular (`window.bar_type = "toolbar"`,
/// `themes.md`, section 6.1): a macOS toolbar. Its buttons are the blocks
/// of the status bar, and open the same menus and pickers, dropped under
/// them; the folder is the title, with what the app says (`messageLine`)
/// and the synchronization under it, as Mail does.
private struct ToolbarView<Messages: View>: View {
    @Environment(AppModel.self) private var model
    let controller: InterfaceController
    let messages: Messages

    var body: some View {
        let listsMail = controller.screen == .mail && (model.listFilter != nil || !model.listedFolderIds.isEmpty)
        HStack(spacing: 8) {
            // The title: the folder, or the list or screen shown; under
            // it, the message of the moment, else the synchronization.
            VStack(alignment: .leading, spacing: 1) {
                if listsMail, model.listFilter == nil {
                    HStack(spacing: 4) {
                        Text(model.currentFolderName).font(.interfaceBar(15, weight: .bold))
                        Image(systemName: "chevron.down").font(.system(size: barScaled(8), weight: .semibold))
                            .foregroundStyle(Palette.muted)
                    }
                    .contentShape(Rectangle())
                    .menuAnchor(.goto, controller, barType: "toolbar")
                    .onTapGesture { controller.openMenuFromStatusBar(.goto) }
                    .uiTarget("status-folder", barType: "toolbar")
                } else {
                    Text(controller.locationLabel).font(.interfaceBar(15, weight: .bold)).lineLimit(1)
                }
                HStack(spacing: 6) {
                    messages
                    if let sync = model.syncLine {
                        Text(sync).foregroundStyle(Palette.muted).lineLimit(1).uiTarget("status-sync", barType: "toolbar")
                    }
                    if !model.marked.isEmpty {
                        Text(L("%lld marked", model.marked.count)).foregroundStyle(Palette.markedBar)
                    }
                }
                .font(.interfaceBar(10))
            }
            .padding(.trailing, barScaled(12))
            // Empty: a click there reaches the drag area under the toolbar
            // (`clickBarBackground`). Its place, for the UI scripts.
            Spacer(minLength: 8).uiTarget("status-empty", barType: "toolbar")
            // The buttons, in the order of the theme (`window.blocks`); the
            // folder is the title, the room left is
            // before the buttons already.
            let items = StatusLayout.items(Look.theme.window.blocks.filter { $0 != "folder" }) { draws($0, listsMail) }
            ForEach(Array(items.enumerated()), id: \.offset) { _, item in
                button(item, listsMail: listsMail)
            }
        }
        .font(.interfaceBar(12))
        .padding(.horizontal, 12)
        .padding(.leading, model.titleBarMode == .merged ? Look.mergedButtonsWidth : 0)
        .frame(height: Look.mergedBarHeight)
        // Its empty parts move the window, as a title bar does.
        .background(ZStack { Palette.statusBar; WindowDragArea(onClick: controller.clickBarBackground) })
        .overlay(alignment: .bottom) { Palette.separator.frame(height: 1) }
    }
}

/// A button of the toolbar: a real button (keyboard focus, accessibility,
/// and a click AppKit delivers like any other), drawn as its block.
extension ToolbarView {
    /// Whether the block `name` has something to show now (as in the
    /// status bar, `StatusBarView.draws`).
    func draws(_ name: String, _ listsMail: Bool) -> Bool {
        switch name {
        case "help": true
        case "account": listsMail && model.listFilter == nil && controller.hasSeveralAccounts
        case "attributes": listsMail
        case "view", "ai": controller.screen == .mail
        case "move", "save": controller.screen == .mail && controller.directTargets.isEmpty
        default: false
        }
    }

    /// One element of the toolbar: a button, or a gap between two groups.
    @ViewBuilder
    func button(_ item: String, listsMail: Bool) -> some View {
        switch item {
        case StatusLayout.gap:
            Color.clear.frame(width: barScaled(6), height: 1)
        case "account":
            ToolbarItem(action: { controller.openPickerFromStatusBar(.accounts) }) {
                StatusBlock(icon: "at", text: model.scopeName, key: controller.keyLabel(.pickerAccounts))
            }
            .uiTarget("status-account", barType: "toolbar")
        case "attributes":
            ToolbarItem(action: { controller.openPickerFromStatusBar(.attributes) }) {
                StatusBlock(icon: "line.3.horizontal.decrease", text: "", key: controller.keyLabel(.pickerAttributes),
                            label: L("Attributes"))
            }
            .uiTarget("status-attributes", barType: "toolbar")
            ForEach(model.attributes.chips) { chip in
                Text(chip.label + " ×")
                    .lineLimit(1)
                    .foregroundStyle(Palette.accentText)
                    .toolbarCapsule()
                    .onTapGesture { model.attributes.remove(chip.id) }
                    .uiTarget("chip-\(chip.id)")
            }
        case "view", "move", "save":
            let p = InterfaceController.Prefix(name: item)
            ToolbarItem(action: { controller.openMenuFromStatusBar(p) }) {
                StatusBlock(icon: p.icon, text: "", key: controller.menuKeyLabel(p), label: p.title)
                    .menuAnchor(p, controller, barType: "toolbar")
            }
            .uiTarget("status-\(p.uiID)", barType: "toolbar")
        case "ai":
            ToolbarItem(action: { controller.openAIFromStatusBar() }) {
                StatusBlock(icon: "sparkles", text: "", key: controller.aiKeyLabel(composing: model.composer != nil), label: L("AI"))
            }
            .help(L("AI assistance"))
            .uiTarget("status-ai", barType: "toolbar")
        case "help":
            ToolbarItem(action: { controller.toggleHelpFromStatusBar() }) {
                StatusBlock(icon: "questionmark.circle", text: "", key: controller.keyLabel(.helpShow))
            }
            .help(L("Keys"))
            .uiTarget("status-help", barType: "toolbar")
        default:
            EmptyView()
        }
    }
}

private struct ToolbarItem<Label: View>: View {
    let action: () -> Void
    @ViewBuilder let label: () -> Label

    var body: some View {
        Button(action: action, label: label).buttonStyle(.plain)
    }
}

extension View {
    /// A button of the toolbar: a capsule around its icon and its name.
    @MainActor
    fileprivate func toolbarCapsule() -> some View {
        padding(.horizontal, barScaled(9))
            .frame(height: barScaled(24))
            .background(Capsule().fill(Palette.popup))
            .overlay(Capsule().stroke(Palette.separator, lineWidth: 1))
            .contentShape(Capsule())
    }
}

/// One element of the status bar: an SF Symbol, a name, and a reminder of
/// the key that opens it (`ui.md`, status bar).
extension View {
    /// Records the left edge of a status bar block, where the menu of
    /// `prefix` drops down from.
    ///
    /// `barType`: the command of the window the block belongs to (the status
    /// bar or the toolbar): a block of the other one, still laid out while
    /// the window is rebuilt in a new theme, does not move the menu.
    ///
    /// The theme the block was drawn in is kept too (`ThemeStore.revision`):
    /// when the theme changes, the blocks of the previous one, clean's for
    /// geek's, still report a place as they go away, often 0, which sent the
    /// menu to the left edge of the window.
    @MainActor
    fileprivate func menuAnchor(_ prefix: InterfaceController.Prefix, _ controller: InterfaceController,
                                barType: String = "status_bar") -> some View {
        let revision = ThemeStore.shared.revision
        return background(GeometryReader { geo in
            Color.clear
                .onAppear { place(prefix, controller, barType, revision, geo.frame(in: .global)) }
                .onChange(of: geo.frame(in: .global)) { _, f in place(prefix, controller, barType, revision, f) }
        })
    }

    @MainActor
    private func place(_ prefix: InterfaceController.Prefix, _ controller: InterfaceController, _ barType: String,
                       _ revision: Int, _ frame: CGRect) {
        guard ThemeStore.shared.current.window.barType == barType, revision == ThemeStore.shared.revision,
              frame.width > 0 else { return }
        controller.menuBlockX[prefix] = frame.minX
    }
}

private struct StatusBlock: View {
    /// `nil`: no icon (a move target of Quarantine, after the group's arrow).
    let icon: String?
    let text: String
    let key: String
    /// The name of a block of action, said beside its icon when the theme
    /// asks for it (`window.block_labels`).
    var label: String? = nil
    /// Where the list is (the account, the folder): in bold in a theme of
    /// pills (`window.block_style`), the actions in grey.
    var strong = false
    /// Its menu or picker is open: highlighted in a theme of pills.
    var active = false

    var body: some View {
        let pill = Look.pillBlocks && !Look.toolbar
        HStack(spacing: barScaled(pill ? 5 : 4)) {
            if let icon {
                Image(systemName: icon)
                    .font(.system(size: barScaled(pill ? 11 : 10), weight: pill ? .medium : .semibold))
                    .foregroundStyle(pill && strong ? Palette.text : Palette.muted)
            }
            if !text.isEmpty {
                Text(text).lineLimit(1)
                    .fontWeight(pill && strong ? .semibold : nil)
                    .foregroundStyle(pill && !strong ? Palette.muted : Palette.text)
            }
            if text.isEmpty, Look.blockLabels, let label {
                Text(label).lineLimit(1).foregroundStyle(pill ? (active ? Palette.text : Palette.muted) : Palette.text)
            }
            // The reminder of the key, unless the theme leaves it out
            // (`window.key_hints`): the key works either way.
            if Look.keyHints {
                Text(key)
                    .font(.interfaceBar(9))
                    .foregroundStyle(Palette.muted)
                    .padding(.horizontal, barScaled(3))
                    // Square corners: the geek theme has no rounded shapes (`ui.md`).
                    .overlay(Rectangle().stroke(Palette.separator, lineWidth: 1))
            }
        }
        // A pill (clean): room around the block, highlighted while its
        // menu is open, as a tab of a web page.
        .padding(.horizontal, pill ? barScaled(9) : 0)
        .padding(.vertical, pill ? barScaled(4) : 0)
        .background(Capsule().fill(pill && active ? Palette.cursor : Color.clear))
        .contentShape(Rectangle())
        // A button of the toolbar in regular (`window.bar_type`).
        .modifier(ToolbarButton(active: Look.toolbar))
    }
}

/// The capsule of a toolbar button, or nothing.
private struct ToolbarButton: ViewModifier {
    let active: Bool
    func body(content: Content) -> some View {
        if active { content.toolbarCapsule() } else { content }
    }
}

// MARK: - Columns

/// 1, 2 or 3 columns (`ui.md`, `v1` `v2` `v3`), separated by 1-pixel lines;
/// the separator before the mail can be dragged.
private struct MailColumns: View {
    @Environment(AppModel.self) private var model
    let controller: InterfaceController

    var body: some View {
        GeometryReader { geo in
            let folderWidth: CGFloat = controller.columns == 3 ? 190 : 0
            let rest = geo.size.width - folderWidth
            let listWidth = controller.columns == 1 ? rest : max(220, min(rest - 260, rest * controller.listFraction))
            HStack(spacing: 0) {
                if controller.columns == 1, controller.reading, model.selectedThreadId != nil || model.composer != nil {
                    // One column, Return pressed: the mail takes the whole
                    // window until Escape. j / k still move the cursor of
                    // the hidden list, so the next mail is one key away.
                    // When the list empties (last mail archived), the list
                    // comes back rather than an empty reader.
                    ReaderColumn(controller: controller)
                } else {
                    columns(folderWidth: folderWidth, rest: rest, listWidth: listWidth)
                }
            }
        }
        .coordinateSpace(name: "columns")
    }

    /// The usual layout: folders (3 columns), list, and mail (2 or 3).
    @ViewBuilder
    private func columns(folderWidth: CGFloat, rest: CGFloat, listWidth: CGFloat) -> some View {
        if controller.columns == 3 {
            FolderColumn(controller: controller).frame(width: folderWidth)
            Look.separator.frame(width: 1)
        }
        ThreadColumn(controller: controller).frame(width: listWidth).clipped()
        if controller.columns > 1 {
            Look.separator.frame(width: 1)
                // A wider invisible zone makes the line easy to grab.
                .overlay(Color.clear.frame(width: 7).contentShape(Rectangle()).onHover { inside in
                    if inside { NSCursor.resizeLeftRight.push() } else { NSCursor.pop() }
                })
                .gesture(DragGesture(minimumDistance: 1, coordinateSpace: .named("columns")).onChanged { g in
                    controller.listFraction = Double(max(0.15, min(0.7, (g.location.x - folderWidth) / max(rest, 1))))
                })
            ReaderColumn(controller: controller)
        }
    }
}

/// Third column: the folders of the current view, under a single title:
/// "all" (the unified view) or the account's name (`ui.md`).
private struct FolderColumn: View {
    @Environment(AppModel.self) private var model
    let controller: InterfaceController

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Text(model.scopeName)
                    .foregroundStyle(Palette.muted)
                    .padding(.top, scaled(4)).padding(.bottom, scaled(2)).padding(.horizontal, Look.sidebarTextInset)
                    .contentShape(Rectangle())
                    .onTapGesture { controller.openPickerFromStatusBar(.accounts) }
                let folders = controller.scopeFolders
                ForEach(Array(folders.enumerated()), id: \.element.id) { i, f in
                    // The current folder on a fill of the accent (regular):
                    // its name, count and key take the text color of the
                    // selection, white, or they would be lost in the blue.
                    let onSelection = controller.isCurrent(f) && Look.strongSelection
                    let id = f.role.map { Role.name($0) } ?? f.virtual?.id ?? f.name
                    HStack(spacing: 6) {
                        // The key of the folder in the menu `;` (`goto.<role>`):
                        // first, or after the count when the theme puts the
                        // keys on the right (`layout.menu_keys`).
                        if !Look.menuKeysRight {
                            Text(controller.gotoKeyLabel(f)).foregroundStyle(onSelection ? Palette.selectionText : Palette.muted)
                        }
                        Group {
                            if f.virtual != nil {
                                VirtualFolderLabel(name: f.name, color: onSelection ? Palette.selectionText : Palette.accentText)
                            } else {
                                Text(f.name).foregroundStyle(onSelection ? Palette.selectionText : Palette.text)
                            }
                        }
                        .lineLimit(1)
                        .uiTarget("folder-name-\(id)")
                        Spacer(minLength: scaled(8))
                        if f.unread > 0 {
                            // As in the menu `;`: a pill beside a key on
                            // the right, a bare number when the key is first.
                            if KeyMenu.detailAsBadge(keysRight: Look.menuKeysRight, muted: false) {
                                UnreadBadge(count: "\(f.unread)", onAccent: onSelection)
                                    .uiTarget("folder-badge-\(id)")
                            } else {
                                Text("\(f.unread)").foregroundStyle(onSelection ? Palette.selectionText : Palette.unread)
                            }
                        }
                        if Look.menuKeysRight {
                            Text(controller.gotoKeyLabel(f))
                                .foregroundStyle(onSelection ? Palette.selectionText.opacity(0.8) : Palette.muted)
                                .uiTarget("folder-key-\(id)")
                                // More room after a count, as in the menu `;`.
                                .padding(.leading, f.unread > 0 ? scaled(KeyMenu.keyGapAfterBadge) : 0)
                        }
                    }
                    // The text inside the highlight, away from its rounded
                    // edges (`Look.sidebarTextInset`).
                    .padding(.horizontal, Look.sidebarTextInset)
                    .frame(height: Look.rowHeight)
                    .rowFill(controller.isCurrent(f) ? Palette.cursor : nil)
                    // The whole row is clickable, not only its text.
                    .contentShape(Rectangle())
                    .onTapGesture { controller.open(f) }
                    // In English whatever `[folder_names]` says, so that a
                    // scenario does not depend on the names of a file; a
                    // virtual folder by its id (`folder-all_mail`), as a
                    // target of a scenario has no space.
                    .uiTarget("folder-\(id)")
                    // The same blocks as the ; menu.
                    .padding(.top, i > 0 && folders[i - 1].group != f.group ? scaled(10) : 0)
                }
                ColumnGroups(controller: controller)
            }
        }
        .scrollIndicators(Look.scrollIndicators)
        // Its own background (`colors.sidebar`): the background of the
        // window in geek.
        .background(Palette.sidebar)
        .uiTarget("folder-column")
    }
}

/// The contact groups, under the folders (`ui.md`, "Groupes dans la colonne
/// des dossiers"). A click on a group narrows the current list to the mails
/// of its members (the Group… attribute, chip in the status bar), a second
/// click removes it; ⌘-click excludes it. Several groups combine by "or".
/// With more groups than room, the most relevant ones are shown
/// (`GroupRanking`) and a last row "N more…" opens the Group… panel, which
/// lists them all. No groups: no block at all, not even its title.
private struct ColumnGroups: View {
    @Environment(AppModel.self) private var model
    let controller: InterfaceController

    var body: some View {
        let (rows, hidden) = controller.columnGroups
        if !rows.isEmpty {
            Text(L("Groups"))
                .foregroundStyle(Palette.muted)
                .padding(.top, scaled(14)).padding(.bottom, scaled(2)).padding(.horizontal, Look.sidebarTextInset)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
                .onTapGesture { controller.openGroupPanel() }
                .uiTarget("groups-title")
            ForEach(rows) { g in
                let choice = model.attributes.groupChoice(g.id)
                // Highlighted on a fill of the accent (regular): white.
                let on = controller.isHighlighted(g)
                HStack(spacing: 6) {
                    // Where a folder has its letter: "•" when the group
                    // filters the list, "−" when it is excluded, as in the
                    // Group… panel.
                    Text(choice == true ? "•" : choice == false ? "−" : "@")
                        .foregroundStyle(Look.ink(choice == nil ? Palette.muted : Palette.accentText, selected: on))
                    Text(g.name)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .foregroundStyle(Look.ink(choice == nil ? Palette.text : Palette.accentText, selected: on))
                        .strikethrough(choice == false)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, Look.sidebarTextInset)
                .frame(height: Look.rowHeight)
                // The same grey row as the current folder above.
                .rowFill(controller.isHighlighted(g) ? Palette.cursor : nil)
                .contentShape(Rectangle())
                .onClick { controller.clickGroup(g, $0) }
                .uiTarget("group-\(g.name)")
                .accessibilityElement(children: .combine)
                .accessibilityLabel(choice == true ? L("group %@, filtering the list", g.name)
                                    : choice == false ? L("group %@, excluded from the list", g.name)
                                    : L("group %@", g.name))
                .accessibilityAddTraits(.isButton)
            }
            if hidden > 0 {
                Text(L("%lld more…", hidden))
                    .foregroundStyle(Palette.muted)
                    .padding(.horizontal, Look.sidebarTextInset)
                    .frame(maxWidth: .infinity, minHeight: scaled(20), alignment: .leading)
                    .contentShape(Rectangle())
                    .onTapGesture { controller.openGroupPanel() }
                    .uiTarget("groups-more")
            }
        }
    }
}

/// The mail list. (`/` used to show its own results view; it now filters
/// this same list, `ui.md`.)
private struct ThreadColumn: View {
    let controller: InterfaceController

    var body: some View {
        // Its own background (`colors.list`): the window's in geek.
        ThreadRows(controller: controller).background(Palette.list)
    }
}

private struct ThreadRows: View {
    @Environment(AppModel.self) private var model
    let controller: InterfaceController

    var body: some View {
        let cursor = model.selectedThreadId
        let marked = model.marked
        // In "Everything", the folder of each row (nil elsewhere).
        let folderLabels = model.everythingFolderLabels
        // Read once per redraw, like the labels, not row by row.
        let showsKind = model.listShowsSenderKind
        // The column widths depend on the list width only, never on a row's
        // content: every row gets the same layout and the columns line up.
        GeometryReader { geo in
            let layout = RowLayout(width: geo.size.width)
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        // Swift note: `enumerated()` pairs each thread with
                        // its place, for the alternate rows of a theme
                        // (`layout.zebra`); the id stays the thread's.
                        ForEach(Array(model.threads.enumerated()), id: \.element.id) { index, t in
                            ThreadRow(
                                thread: t, cursor: t.id == cursor, marked: marked.contains(t.id), layout: layout, index: index,
                                folder: folderLabels.map { $0[t.folderId] ?? .init(folder: "", account: nil) },
                                showsKind: showsKind, suggestion: model.aiSuggestions[t.id], names: model.folderNames)
                                .contentShape(Rectangle())
                                // Click, ⌘-click, ⇧-click (`clickThread`).
                                .onClick { controller.clickThread(t.id, $0) }
                                .uiTarget("thread-\(t.id)")
                                .id(t.id)
                                .task { await model.loadMoreIfNeeded(after: t) }
                        }
                    }
                }
                .scrollIndicators(Look.scrollIndicators)
                // How many rows fit, for ctrl-d / ctrl-u (half a page). Read
                // again when the window or the text size changes.
                .onChange(of: geo.size.height, initial: true) { _, h in
                    controller.listVisibleRows = InterfaceController.visibleRows(height: h, rowHeight: Look.rowHeight)
                }
                // For the UI scripts: whether the rows have room for the
                // account (`row_folder_with_account`).
                .onChange(of: geo.size.width, initial: true) { _, w in
                    if UIScript.isActive { UIScript.rowFolderWithAccount = RowLayout(width: w).showsAccount }
                }
                // Keep the cursor visible, without animation: `scrollTo`
                // with no anchor scrolls only as much as needed.
                .onChange(of: model.selectedThreadId) { _, id in
                    if let id { proxy.scrollTo(id) }
                }
                // After a rebuild (new text size, new layout), bring the
                // cursor back into view.
                .onAppear { if let id = model.selectedThreadId { proxy.scrollTo(id) } }
                .overlay {
                    if model.threads.isEmpty {
                        // During the launch, what is happening (the core
                        // starting, the list loading) rather than "No
                        // mail", which an empty list would otherwise say
                        // of a full mailbox.
                        Text(model.startupPhase.message ?? L("No mail")).foregroundStyle(Palette.muted)
                    }
                }
            }
        }
    }
}

/// Fixed column widths of the list rows, computed from the list width.
///
/// In a font of fixed pitch (geek), in whole characters: every column of
/// every row lines up. In a proportional font (clean, regular), the date
/// and the count are measured on the texts they can hold, so that they are
/// as narrow as they can be without cutting one (`TextMeasure`,
/// `themes.md`, section 7).
@MainActor
struct RowLayout: Equatable {
    /// Font size of the rows, written for the default text size.
    static let fontSize: CGFloat = 12
    /// Width of "0": the unit of the gaps and of the columns counted in
    /// characters.
    let char: CGFloat
    /// Date: 8 characters ("14/09/21", "16 sept."), or, in a proportional
    /// font, the widest date the list can show.
    let date: CGFloat
    /// Sender: about a quarter of the list, between 8 and 24 characters.
    let sender: CGFloat
    /// Message count: 3 digits, right-aligned.
    let count: CGFloat
    /// The unread dot: one character, or the width of "●" in a
    /// proportional font, where it is wider than a digit.
    let dot: CGFloat

    /// Width of the list, to know what room the subject has.
    let width: CGFloat

    /// Folder column of "Everything": 10 characters, the longest folder
    /// names ("Newsletter", "Quarantine"); 18 with the account.
    func folder(withAccount: Bool) -> CGFloat { (withAccount ? 18 : 10) * char }

    /// Paperclip and flag: one icon each, a character and a half.
    var icon: CGFloat { (char * 1.5).rounded(.up) }

    /// Width left to the subject (and tags) in a row with a folder column
    /// of `folderWidth`: the list minus the fixed columns (mark bar, unread
    /// dot, date, sender, paperclip, folder, count, flag), the 8 gaps
    /// between the 9 columns and the trailing padding.
    func subjectWidth(folderWidth: CGFloat) -> CGFloat {
        width - 2 - dot - date - sender - icon - folderWidth - count - icon - 8 * char - 6
    }

    /// The account fits after the folder ("Inbox · perso") only if the
    /// subject keeps 30 characters: in a narrow list the subject matters
    /// more, and the reader shows the account anyway.
    var showsAccount: Bool { subjectWidth(folderWidth: folder(withAccount: true)) >= 30 * char }

    /// The AI's suggested target is cut short ("→ Newsl…") when the subject
    /// would keep fewer than 40 characters beside its full name: in two
    /// columns, the subject says more than the target, which the status
    /// bar names in full.
    var narrowSuggestion: Bool { subjectWidth(folderWidth: 12 * char) < 40 * char }

    /// `scale`: the text size factor (`TextSize.scale`); `measure`: the
    /// font of the rows, by default the interface font of the theme.
    init(width: CGFloat, scale: CGFloat? = nil, measure: TextMeasure? = nil) {
        let m = measure ?? TextMeasure.interface(Self.fontSize * (scale ?? TextSize.shared.scale))
        self.width = width
        char = m.char
        if m.fixedPitch {
            date = 8 * char
        } else {
            date = RowLayout.dateWidth(m)
        }
        let chars = (width / char * 0.26).rounded(.down)
        sender = min(24, max(8, chars)) * char
        count = m.width("999")
        dot = m.fixedPitch ? char : m.width(RowLayout.unreadDot)
    }

    /// The mark of an unread conversation.
    static let unreadDot = "●"

    /// The widest date of the list (`Format.geekListDate`): a time today,
    /// a day and month this year, a short date before, on the sample dates
    /// that give the longest texts (`Format.samples`), in the app's locale.
    static func dateWidth(_ m: TextMeasure, dates f: Format.Dates = Format.dates) -> CGFloat {
        let samples = Format.samples(calendar: f.geekDayMonth.calendar, timeZone: f.geekDayMonth.timeZone)
        let texts = samples.flatMap { [f.geekTime.string(from: $0), f.geekDayMonth.string(from: $0), f.geekShort.string(from: $0)] }
        return texts.map { m.width($0) }.max() ?? 8 * m.char
    }
}

/// The sender column of a list row (`ui.md`, "Liste").
enum SenderColumn {
    /// The arrow shown before the correspondent when the user wrote the
    /// latest mail. A symbol, the same in every language.
    static let mark = "→"

    /// What the column says: "Alice", or "→ Alice" when the user spoke
    /// last.
    static func plain(_ thread: ThreadSummary) -> String {
        thread.lastIsMine ? "\(mark) \(thread.correspondentName)" : thread.correspondentName
    }

    /// The same, the arrow muted: it marks the row, the name is what one
    /// reads.
    ///
    /// Swift note: an `AttributedString` carries styles on ranges of its
    /// text; a `Text` built from it draws each range in its own style.
    static func text(_ thread: ThreadSummary) -> AttributedString {
        let name = AttributedString(thread.correspondentName)
        guard thread.lastIsMine else { return name }
        var arrow = AttributedString("\(mark) ")
        arrow.foregroundColor = Palette.muted
        return arrow + name
    }

    /// What VoiceOver reads: the name, then that the user wrote last, then
    /// (`showsKind`, in Quarantine) whether a person or a robot wrote.
    static func spoken(_ thread: ThreadSummary, showsKind: Bool = false) -> String {
        let name = thread.lastIsMine ? L("%@, you wrote last", thread.correspondentName) : thread.correspondentName
        guard showsKind else { return name }
        return switch thread.correspondentKind {
        case "person": L("%@, written by a person", name)
        case "robot": L("%@, written by a robot", name)
        default: name
        }
    }

    /// The SF Symbol drawn before the name in Quarantine (`ui.md`,
    /// "Quarantine"): a silhouette for a person, a gear for a robot.
    /// `nil` without a verdict (no readable sender, or the catch-up has not
    /// read the mail yet): the core's filter ranks such mails with the
    /// robots, but a gear would claim something nobody computed, so the
    /// place stays blank. Symbols, not words: the same in every language
    /// and one character and a half wide (SF Symbols has no robot).
    static func kindSymbol(_ thread: ThreadSummary) -> String? {
        switch thread.correspondentKind {
        case "person": "person.fill"
        case "robot": "gearshape.fill"
        default: nil
        }
    }
}

/// One list row: mark bar, unread dot, date, sender, paperclip, subject and
/// tags, (in "Everything") folder, count, flag. Every column but the
/// subject has a fixed width.
///
/// The paperclip says the conversation has attachments, without a number,
/// and sits before the subject: next to the message count at the end of
/// the row, "📎 3" would read as three attachments (`ui.md`, "Liste").
private struct ThreadRow: View {
    let thread: ThreadSummary
    let cursor: Bool
    let marked: Bool
    let layout: RowLayout
    /// Its place in the list, for the alternate rows (`layout.zebra`).
    var index = 0
    /// Folder of the row, shown in "Everything" only (`nil` elsewhere).
    var folder: AppModel.FolderLabel? = nil
    /// The icon person / robot before the name (Quarantine only,
    /// `AppModel.listShowsSenderKind`).
    var showsKind = false
    /// The target the AI suggests for the conversation (`a` `s`).
    var suggestion: AISuggestion? = nil
    /// The names of the folders, for the suggestion (`FolderNames`).
    var names: FolderNames = .english

    var body: some View {
        // The row under the cursor on a fill of the accent (regular): every
        // text and icon in the selection's text color (`Look.ink`).
        let on = cursor
        HStack(spacing: layout.char) {
            (marked ? Palette.markedBar : Color.clear).frame(width: 2)
            Text(thread.unreadCount > 0 ? RowLayout.unreadDot : " ").foregroundStyle(Look.ink(Palette.unread, selected: on)).frame(width: layout.dot)
            Text(Format.geekListDate(thread.dateLastReceived)).foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                .frame(width: layout.date, alignment: .leading)
            // The person the conversation is with, not the user who
            // answered last; "→" says the user spoke last and the
            // conversation waits on them (`ui.md`, "Liste"). One `Text`,
            // so a long name is cut after the arrow, never the arrow.
            //
            // In Quarantine an icon comes first, inside the column: the
            // columns after it stay where they are, and the name gives up
            // the room. A row without verdict keeps the blank place, so
            // every name starts at the same x.
            HStack(spacing: layout.char / 2) {
                if showsKind {
                    Group {
                        if let symbol = SenderColumn.kindSymbol(thread) {
                            Image(systemName: symbol)
                                .font(.system(size: scaled(10), weight: .medium))
                                // The person in the accent colour: among
                                // the robots of Quarantine, the mail that
                                // most likely deserves the Inbox stands out.
                                .foregroundStyle(Look.ink(thread.correspondentKind == "person" ? Palette.accentText : Palette.muted, selected: on))
                        } else {
                            Color.clear
                        }
                    }
                    .frame(width: layout.icon)
                    // Read once, with the name (`spoken`), not as an image.
                    .accessibilityHidden(true)
                }
                Text(SenderColumn.text(thread))
                    .fontWeight(thread.unreadCount > 0 ? .bold : .regular)
                    .foregroundStyle(Look.ink(Palette.text, selected: on))
                    .lineLimit(1)
                Spacer(minLength: 0)
            }
            .frame(width: layout.sender, alignment: .leading)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(SenderColumn.spoken(thread, showsKind: showsKind))
            Group {
                if thread.hasAttachments {
                    Image(systemName: "paperclip")
                        .font(.system(size: scaled(11), weight: .medium))
                        .foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                        .accessibilityLabel(L("with attachments"))
                } else {
                    Color.clear
                }
            }
            .frame(width: layout.icon)
            // The subject takes the remaining width; tags follow it and are
            // cut first.
            HStack(spacing: layout.char) {
                Text(thread.subject.isEmpty ? L("(no subject)") : thread.subject).layoutPriority(1)
                    .foregroundStyle(Look.ink(Palette.text, selected: on))
                ForEach(thread.tags, id: \.self) { Text("#\($0)").foregroundStyle(Look.ink(Palette.accentText, selected: on)) }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            // Muted, fixed width. The account follows only when the
            // subject keeps enough room (`RowLayout.showsAccount`); the
            // reader always shows it.
            if let folder {
                let withAccount = folder.account != nil && layout.showsAccount
                Text(withAccount ? folder.text : folder.folder).foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                    .frame(width: layout.folder(withAccount: withAccount), alignment: .leading)
            }
            // The AI's suggestion, before the count: the target's name in
            // the accent colour, after an arrow, as the move it proposes.
            if let s = suggestion {
                Text(AI.suggestionTag(names.name(s.target), narrow: layout.narrowSuggestion))
                    .foregroundStyle(Look.ink(Palette.accentText, selected: on))
                    .fixedSize()
                    .accessibilityLabel(L("AI suggests %@", names.name(s.target)))
            }
            Text(thread.messageCount > 1 ? "\(thread.messageCount)" : "").foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                .frame(width: layout.count, alignment: .trailing)
            // The flag: an icon in its own colour, apart from the unread
            // dot's, at the end of the row.
            Group {
                if thread.isFlagged {
                    Image(systemName: "flag.fill")
                        .font(.system(size: scaled(11), weight: .semibold))
                        .foregroundStyle(Look.ink(Palette.flag, selected: on))
                        .accessibilityLabel(L("flagged"))
                } else {
                    Color.clear
                }
            }
            .frame(width: layout.icon)
        }
        .font(.interface(RowLayout.fontSize, weight: .regular))
        .lineLimit(1)
        .padding(.trailing, 6)
        .frame(height: Look.rowHeight)
        .rowFill(cursor ? Palette.cursor : marked ? Palette.marked : nil, index: index)
    }
}

// MARK: - Reader

/// The title line of the reader: the subject, then the controls of the
/// mail (unsubscribe, blocked pixels, text | html | images, folder).
///
/// On one line when the whole subject fits beside the controls, otherwise
/// on two: the subject alone, then the controls aligned on the right. The
/// choice is measured, not guessed: the font size (⌘+ ⌘−), the length of
/// the subject, the controls this mail has and the width of the reader all
/// count, and it is made again whenever one of them changes.
///
/// Swift note: `ViewThatFits` lays out its children in order and shows the
/// first whose ideal width fits in the space offered. The ideal width of a
/// one-line `Text` is that of its whole text, so the first child fits only
/// when the subject is not cut.
struct ReaderTitleBar<Controls: View>: View {
    let subject: String
    /// The thread's tags and the groups of the person it is with, after
    /// the subject (`ui.md`, "Tags et groupes dans l'en-tête"); none while
    /// a reply is written.
    var tags: [String] = []
    var groups: [String] = []
    /// The subject on its line and the controls always on the next one,
    /// whatever the room (the reply: its commands are many, and a subject
    /// being typed should not make them jump from one line to two).
    var alwaysTwoLines = false
    /// Shown, muted, in place of an empty subject on two lines.
    var placeholder: String? = nil
    @ViewBuilder let controls: () -> Controls

    var body: some View {
        Group {
            if alwaysTwoLines {
                twoLines
            } else if subject.isEmpty && tags.isEmpty && groups.isEmpty {
                // Nothing to show before the controls (a new mail, its
                // subject not typed yet): the controls alone, on one line.
                // Two lines would open on an empty one.
                HStack(spacing: scaled(8)) {
                    Spacer(minLength: 0)
                    controls()
                }
                .frame(minHeight: scaled(26))
                .onAppear { Self.report(lines: 1) }
            } else {
                subjectAndControls
            }
        }
        // 12 pt on the sides in geek; room on every side in clean, whose
        // title has no line to set it apart (`layout.reader_title_padding`,
        // `Look.readerTitleInsets`).
        .padding(Look.readerTitleInsets)
        .uiTarget("reader-title")
    }

    /// The subject and the controls, on one line or on two.
    private var subjectAndControls: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: scaled(8)) {
                subjectText
                labels
                Spacer(minLength: scaled(16))
                controls()
            }
            // At least 26 pt; more when the theme's subject is taller
            // (`fonts.title_scale`), rather than clip it.
            .frame(minHeight: scaled(26))
            .onAppear { Self.report(lines: 1) }
            twoLines
        }
    }

    /// The subject alone, then the controls aligned on the right.
    private var twoLines: some View {
        // The gap between the two lines grows with the room around the
        // title (`Look.readerTitleLineGap`): none in geek, 6 pt in clean.
        VStack(alignment: .leading, spacing: Look.readerTitleLineGap) {
            // At least 24 pt: a subject bigger than geek's (clean's is
            // 1.7 times as big) takes its own height, or it would run
            // into the line of the controls.
            subjectText
                .frame(maxWidth: .infinity, alignment: .leading)
                .frame(minHeight: scaled(24))
            // On two lines, the labels open the line of the controls:
            // the subject keeps the whole width.
            HStack(spacing: scaled(8)) {
                labels
                Spacer(minLength: 0)
                controls()
            }
            .frame(height: scaled(22))
            .uiTarget("reader-title-controls")
            .padding(.bottom, scaled(2))
        }
        .onAppear { Self.report(lines: 2) }
    }

    // Selectable like the body (the text reader is an NSTextView): the
    // subject is often what one copies to search for it or quote it.
    // Swift note: `.textSelection(.enabled)` lets the user drag over a
    // SwiftUI `Text` and copy it with ⌘C; ⌘ shortcuts are left to AppKit
    // by `InterfaceController.handle`. On two lines, a subject longer than the
    // whole reader is cut at the end; the tooltip shows it whole.
    private var subjectText: some View {
        // Its size follows the theme (`fonts.title_scale`: 1 in geek).
        let shown = subject.isEmpty ? (placeholder ?? "") : subject
        return Text(shown).font(.interface(13 * Look.titleScale, weight: .bold)).lineLimit(1)
            .foregroundStyle(subject.isEmpty ? Palette.muted : Palette.text)
            .textSelection(.enabled)
            .help(subject)
            .uiTarget("reader-subject")
    }

    /// `#tag` as in the list rows, `@group` as in the folder column: the
    /// same marks say the same things everywhere. Never cut: a label is
    /// short, and the layout goes to two lines rather than hide one.
    ///
    /// Swift note: `@ViewBuilder` lets a property return different views
    /// from an `if`; an empty `HStack` would still take a spacing slot.
    @ViewBuilder
    private var labels: some View {
        if !tags.isEmpty || !groups.isEmpty {
            HStack(spacing: scaled(6)) {
                ForEach(tags, id: \.self) { Text("#\($0)") }
                ForEach(groups, id: \.self) { Text("@\($0)") }
            }
            .font(.interface(12))
            .foregroundStyle(Palette.accentText)
            .lineLimit(1)
            .fixedSize()
            .accessibilityElement(children: .combine)
            .help(LabelChoices.headerHelp(tags: tags, groups: groups))
        }
    }

    /// In UI script mode, which layout is shown (`title_lines`).
    private static func report(lines: Int) {
        if UIScript.isActive { UIScript.titleLines = lines }
    }
}

/// The open thread, as text (iA Writer Mono V) or as sanitized HTML,
/// following the choice remembered for the sender.
private struct ReaderColumn: View {
    @Environment(AppModel.self) private var model
    let controller: InterfaceController

    var body: some View {
        if model.composer != nil {
            // The reply being written takes the reader's place
            // (`reponse.md`, section 6); Escape brings the mail back.
            ComposerView(controller: controller)
        } else if let thread = model.openThread, thread.id == model.selectedThreadId {
            let state = controller.readerState
            // The address book's icons are part of the blocks: the text is
            // rebuilt when the cards arrive (`TextReader.refresh`).
            let blocks = ThreadDisplay.blocks(thread, state, book: model.senderBook)
            VStack(spacing: 0) {
                // The subject, then the controls of the mail; on two lines
                // when the subject would not fit whole beside them
                // (`ReaderTitleBar`).
                ReaderTitleBar(subject: thread.subject, tags: thread.labels.tags, groups: thread.labels.groups) {
                    titleControls(state)
                }
                Look.separator.frame(height: 1)
                if model.bodyFormat != "text" {
                    let page = threadHTML(thread, state)
                    HTMLReader(html: page.html, allowedImages: page.images, inlineImages: page.inline,
                               darkMode: page.darkMode, controller: controller)
                } else {
                    TextReader(blocks: blocks, threadId: thread.id, controller: controller)
                }
            }
        } else {
            Text(L("No mail")).foregroundStyle(Palette.muted).frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    /// The controls of the title line, left to right: `T`, unsubscribe,
    /// blocked pixels, the text | html | images switch, the folder.
    @ViewBuilder
    private func titleControls(_ state: ThreadDisplay.State) -> some View {
        if state.showAll {
            Text("T").foregroundStyle(Palette.accentText)
        }
        // Unsubscribe, left of text | html (`ui.md`,
        // "Désabonnement"); "unsubscribed on …" once done.
        if let offer = model.unsubscribeOffer {
            if let at = offer.senderUnsubscribedAt {
                Text(UnsubscribeDisplay.done(at)).foregroundStyle(Palette.muted)
            } else {
                let label = UnsubscribeDisplay.label(offer, key: controller.keyLabel(.mailUnsubscribe))
                HStack(spacing: scaled(4)) {
                    Text(label.text)
                    if let symbol = label.symbol { Image(systemName: symbol) }
                }
                .foregroundStyle(offer.source == "body" ? Palette.muted : Palette.accentText)
                .padding(.horizontal, scaled(4))
                // Rounded as the rows of the list, for its size
                // (`Look.titleControlRadius`): square in geek.
                .overlay(RoundedRectangle(cornerRadius: Look.titleControlRadius, style: .continuous)
                    .stroke(Palette.separator, lineWidth: 1))
                .contentShape(Rectangle())
                .help(label.help)
                .onTapGesture { model.unsubscribe() }
                .uiTarget("reader-unsubscribe")
            }
        }
        // Tracking pixels the core removed (`ui.md`, "Images distantes"):
        // shown in every display, they are never loaded. An eye struck out
        // and the number, shorter than a sentence on a line that holds the
        // subject; the tooltip and VoiceOver say what it is.
        if model.trackingPixels > 0 {
            HStack(spacing: scaled(3)) {
                Image(systemName: "eye.slash")
                Text("\(model.trackingPixels)")
            }
            .foregroundStyle(Palette.muted)
            .help(L("Tracking pixels removed from this conversation: they are never loaded"))
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(L("pixels blocked: %lld", model.trackingPixels))
        }
        if model.latestReceived?.bodyHtml != nil {
            // The text | html | images switch (`ui.md`); also
            // `vh` and `vi`. Each word is clicked on its own;
            // "images" only when the thread has remote images.
            HStack(spacing: 0) {
                formatSegment("text", Text(" text "))
                formatSegment("html", Text(" html "))
                if model.imagesOffered {
                    formatSegment("images", Text(" " + L("images") + " "))
                }
            }
            .foregroundStyle(Palette.muted)
        }
        // In "Everything", the folder of the mail (and its
        // account in "all"), at the right end of the subject line
        // (`ui.md`): the list mixes every folder.
        if let label = model.selectedThreadFolderLabel {
            Text(label).foregroundStyle(Palette.muted).lineLimit(1)
        }
    }

    /// One word of the text | html | images switch: highlighted when it is
    /// the current display, clicked to choose it.
    private func formatSegment(_ format: String, _ label: Text) -> some View {
        // The highlight is rounded as the rows of the list, for its size
        // (`Look.titleControlRadius`): square in geek.
        // On a fill of the accent (regular), the word of the current
        // display in the selection's text color, white (`Look.ink`).
        let current = model.bodyFormat == format
        return label
            .foregroundStyle(Look.ink(Palette.muted, selected: current))
            .background(current ? Palette.cursor : Color.clear,
                        in: RoundedRectangle(cornerRadius: Look.titleControlRadius, style: .continuous))
            .contentShape(Rectangle())
            .onTapGesture { model.setBodyFormat(format) }
            .uiTarget("format-\(format)")
    }

    /// The thread as one HTML document. Each message is a `<details>`
    /// element (it folds without JavaScript), open when unfolded; its body
    /// is the sanitized HTML, or the visible segments of its text. Quoted
    /// history and signatures marked by the sender's mail app are hidden
    /// unless `T` shows everything.
    ///
    /// In the images mode, the remote images of a message load only when
    /// `RemoteImages.loads` says so (unfolded, sender's choice and
    /// authentication, or asked for this conversation); the others keep a
    /// source the web view ignores. Returns the page, the original URLs it
    /// may load, for the scheme handler (`RemoteImageLoader.allowed`), and
    /// the attachments its embedded images show (`InlineImageLoader`).
    ///
    /// The sender's layout is kept (table widths, padding, alignment,
    /// fonts named in `<font face>`: `ingest.rs`, `LAYOUT_ATTRIBUTES`), but
    /// not its colours: every mail is shown in the theme's colours.
    private func threadHTML(_ thread: ThreadView, _ state: ThreadDisplay.State)
        -> (html: String, images: Set<String>, inline: Set<Int>, darkMode: Bool) {
        func esc(_ s: String) -> String {
            s.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;")
                .replacingOccurrences(of: ">", with: "&gt;")
        }
        // Markers kept by the core's sanitizer (`ingest.rs`, `sanitize_html`).
        let hideQuotes = state.showAll ? "" : """
        .gmail_quote, .gmail_signature, .moz-cite-prefix, .moz-signature, blockquote[type=cite],
        #nm-divRplyFwdMsg, #nm-divRplyFwdMsg ~ *, hr:has(+ #nm-divRplyFwdMsg), #nm-appendonsend ~ * { display: none; }
        """
        // The reader's styles inside each message's frame.
        let frameStyle = MailFrame.baseStyle(hiding: hideQuotes, scale: TextSize.shared.scale)
        // Mails made for a white page, in the dark theme: Dark Reader adapts
        // their colours (`DarkMode`), or, without it, a white sheet.
        let darkMode = DarkMode.available && DarkMode.needed(thread.messages.compactMap(\.bodyHtml))
        let sheet = !DarkMode.available
        var html = """
        <html><head><meta charset="utf-8">
        <meta http-equiv="Content-Security-Policy" content="\(ContentPolicy.header)">
        <style>
        \(Fonts.cssFontFaces)
        :root { color-scheme: light dark; }
        /* The theme font is the page's default; a font the sender named
           wins in its own text. Each HTML message is in its own frame, with
           its own styles (`MailFrame`). */
        body { font-family: \(Fonts.css(ThemeStore.active.fonts.reading)); }
        summary { font-family: \(Fonts.css(ThemeStore.active.fonts.interface)); }
        body { margin: 16px; font-size: \(Int(scaled(14) * CGFloat(ThemeStore.active.fonts.readingScale)))px; }
        summary { font-size: \(Int(scaled(12)))px; color: #888; cursor: pointer; list-style: none; margin: 4px 0; }
        summary::-webkit-details-marker { display: none; }
        details[open] summary { margin-bottom: 12px; }
        summary a.addr { text-decoration: none; color: inherit; }
        /* The recipients, under the sender's line, as in the text reader. */
        summary .rcpt { display: block; margin-top: 2px; }
        summary a.alert { color: #E06C75; }
        @media (prefers-color-scheme: light) { summary a.alert { color: #C0392B; } }
        /* A single mail: a click on its line does nothing, except on the
           address. */
        details.single summary { pointer-events: none; cursor: default; }
        details.single summary a { pointer-events: auto; cursor: pointer; }
        .q { color: #888; border-left: 2px solid #8886; padding-left: 8px; }
        .m { color: #888; font-size: \(Int(scaled(12)))px; }
        pre { font: \(Int(scaled(14) * CGFloat(ThemeStore.active.fonts.readingScale)))px \(Fonts.css(ThemeStore.active.fonts.reading)); white-space: pre-wrap; margin: 0 0 8px; }
        hr { border: 0; border-top: 1px solid #8884; margin: 16px 0; }
        /* Attachment tiles: 3 to 5 per row, as in the text reader. */
        .att { display: grid; gap: 6px; margin: 12px 0;
               grid-template-columns: repeat(auto-fill, minmax(clamp(18%, 24ch, 31%), 1fr)); }
        .att a { display: block; border: 1px solid #8884; padding: 8px; text-decoration: none; color: inherit;
                 font-size: \(Int(scaled(12)))px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
        .att a img { width: \(Int(scaled(13)))px; height: \(Int(scaled(13)))px; vertical-align: -2px; margin-right: 6px; }
        .att a span { display: block; color: #888; font-size: \(Int(scaled(11)))px; }
        summary img { width: \(Int(scaled(11)))px; height: \(Int(scaled(11)))px; vertical-align: -1px; }
        /* The address book's icon keeps its own proportions. */
        summary a.book img { width: auto; }
        /* No scroll bar in the geek theme (ui.md). */
        html { scrollbar-width: none; }
        ::-webkit-scrollbar { display: none; }
        </style></head><body>
        """
        let mode = model.bodyFormat
        let once = model.remoteImagesOnce == thread.id
        // A conversation: each message folds, and shows an arrow.
        let conversation = thread.messages.count > 1
        var images = Set<String>()
        var inline = Set<Int>()
        for (i, m) in thread.messages.enumerated() {
            if i > 0 { html += "<hr>" }
            let unfolded = !conversation || state.showAll || (state.folding[m.id] ?? m.expanded)
            let who = esc(m.fromName ?? m.fromAddr ?? "")
            let date = esc(Format.geekMessageDate(m.dateReceived))
            // One line, as in the text reader (`ThreadDisplay.blocks`): the
            // arrow in a conversation only; folded, the sender's domain
            // last and a snippet; unfolded, the address in full, a link
            // (click: copied; ⌘-click: the list filtered on it), in the
            // alert colour when it deserves a second look.
            let arrow = conversation ? (unfolded ? "▾ " : "▸ ") : ""
            let domain = m.fromName != nil ? m.fromAddr.flatMap(ThreadDisplay.domain).map { " · @" + esc($0) } ?? "" : ""
            var address = ""
            if m.fromName != nil, let addr = m.fromAddr, !addr.isEmpty {
                let alert = ThreadDisplay.senderAlert(m) ? " alert" : ""
                address = " · <a class=\"addr\(alert)\" href=\"nm://sender/\(m.id)\">\(esc(addr))</a>"
            }
            // The address book's icon at the end of the line, as in the text
            // reader: a link that does what `ctrl-c` does.
            if unfolded, let book = model.senderBook[m.id] {
                address += " &nbsp;<a class=\"book\" href=\"nm://book/\(m.id)\" title=\"\(esc(book.label))\">"
                    + "<img src=\"\(Self.symbolDataURI(book.symbol))\" alt=\"\(esc(book.label))\"></a>"
            }
            var summary = unfolded ? "\(arrow)\(who) · \(date)\(address)"
                : "\(arrow)\(who) · \(date)\(domain) — \(esc(ThreadDisplay.snippet(m)))"
            // Unfolded, the recipients on a second line when worth showing,
            // the same rule as the text reader (`ThreadDisplay.recipients`).
            if unfolded, let recipients = ThreadDisplay.recipients(m, state) {
                summary += "<span class=\"rcpt\">\(esc(recipients))</span>"
            }
            if !unfolded && !m.attachments.isEmpty {
                summary += " &nbsp;<img src=\"\(Self.symbolDataURI("paperclip"))\">\(m.attachments.count)"
            }
            // A single mail: its line does not fold it (`single`).
            html += "<details id=\"nm-message-\(m.id)\"\(unfolded ? " open" : "")\(conversation ? "" : " class=\"single\"")><summary>\(summary)</summary>"
            if let body = m.bodyHtml {
                let loading = RemoteImages.loads(m, mode: mode, unfolded: unfolded, once: once)
                if loading { images.formUnion(RemoteImages.sources(in: body)) }
                inline.formUnion(InlineImages.ids(in: body))
                html += MailFrame.wrap(RemoteImages.body(body, loading: loading), base: frameStyle, sheet: sheet)
            } else {
                for (j, seg) in m.segments.enumerated() {
                    if !seg.hidden || state.showAll || state.revealed.contains("\(m.id):\(j)") {
                        let cls = seg.kind == "text" || seg.kind == "forward" ? "" : " class=\"q\""
                        html += "<pre\(cls)>\(esc(seg.text))</pre>"
                    } else {
                        html += "<div class=\"m\">\(esc(ThreadDisplay.label(seg)))</div>"
                    }
                }
            }
            if !m.attachments.isEmpty {
                // The tiles link to nm://attachment/<id>, caught by the
                // web view's navigation delegate (`HTMLReader`).
                html += "<div class=\"att\">"
                for a in m.attachments {
                    let icon = Self.symbolDataURI(AttachmentDisplay.symbol(mimeType: a.mimeType, filename: a.filename))
                    let name = esc(AttachmentDisplay.name(a))
                    html += "<a href=\"nm://attachment/\(a.id)\" title=\"\(name)\"><img src=\"\(icon)\">\(name)"
                        + "<span>\(esc(AttachmentDisplay.size(a.size)))</span></a>"
                }
                html += "</div>"
            }
            html += "</details>"
        }
        return (html + "</body></html>", images, inline, darkMode)
    }


    /// An SF Symbol as a PNG `data:` URL for the HTML reader (a web page
    /// cannot draw SF Symbols), gray like the page's muted text. Rendered
    /// once per symbol.
    private static var symbolURIs: [String: String] = [:]
    private static func symbolDataURI(_ name: String) -> String {
        if let uri = symbolURIs[name] { return uri }
        let config = NSImage.SymbolConfiguration(pointSize: 32, weight: .regular)
            .applying(NSImage.SymbolConfiguration(paletteColors: [NSColor(white: 0.53, alpha: 1)]))
        guard let image = NSImage(systemSymbolName: name, accessibilityDescription: nil)?.withSymbolConfiguration(config),
              let tiff = image.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff),
              let png = rep.representation(using: .png, properties: [:]) else { return "" }
        let uri = "data:image/png;base64," + png.base64EncodedString()
        symbolURIs[name] = uri
        return uri
    }
}

/// Text reader: an AppKit text view, fast with long mails and selectable,
/// without scroll bars. It draws the blocks of `ThreadDisplay`: headers and
/// "···" lines are links (`nm:` scheme) handled by the controller. The
/// controller keeps the scroll view to scroll it from the keyboard (space,
/// shift-space, ctrl-f, ctrl-b, S).
///
/// Swift note: `NSViewRepresentable` wraps an AppKit view for SwiftUI;
/// `makeNSView` builds it once, `updateNSView` refreshes it when the data
/// changes.
private struct TextReader: NSViewRepresentable {
    let blocks: [ThreadDisplay.Block]
    let threadId: Int
    let controller: InterfaceController

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSTextView.scrollableTextView()
        // The theme's scroll bars (`layout.scrollers`): none in geek.
        Look.applyScrollers(to: scroll)
        scroll.hasHorizontalScroller = false
        scroll.drawsBackground = false
        if let text = scroll.documentView as? NSTextView {
            text.isEditable = false
            text.drawsBackground = false
            text.textContainerInset = NSSize(width: 12, height: 12)
            text.delegate = context.coordinator
            // Links keep the colors given below: no blue, no underline.
            text.linkTextAttributes = [.cursor: NSCursor.pointingHand]
        }
        // The tiles depend on the width (columns, names): lay the text out
        // again when the reader is resized (window, columns, list width).
        scroll.contentView.postsFrameChangedNotifications = true
        context.coordinator.observe(scroll.contentView)
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        controller.readerScroll = scroll
        context.coordinator.controller = controller
        // Swift note: the closure keeps a copy of this view (a struct) and a
        // weak reference to the scroll view, to rebuild on resize.
        context.coordinator.rebuild = { [weak scroll] in
            if let scroll { refresh(scroll, context.coordinator) }
        }
        refresh(scroll, context.coordinator)
    }

    /// Rebuilds the text if the blocks, the text size or the layout of the
    /// tiles changed.
    private func refresh(_ scroll: NSScrollView, _ coordinator: Coordinator) {
        guard let text = scroll.documentView as? NSTextView else { return }
        // Attachment tiles per row, from the reader's width (3 to 5).
        // A typical character of the interface font: the name of a file is
        // not known in advance (the width of one, in a font of fixed pitch).
        let charWidth = Double(TextMeasure.interface(12 * TextSize.shared.scale).average)
        // The text keeps a margin of 12 pt, or, when the theme bounds its
        // width (`layout.reader_max_width`), is centered in that width.
        let side = Look.readerMaxWidth.map { max(12, (scroll.contentSize.width - $0) / 2) } ?? 12
        if text.textContainerInset.width != side { text.textContainerInset = NSSize(width: side, height: 12) }
        let width = Double(scroll.contentSize.width - 2 * side)
        let columns = AttachmentDisplay.columns(width: width, charWidth: charWidth)
        // Characters of a name in a tile: its share of the width, minus the
        // icon and the padding.
        let nameChars = max(8, Int(width / Double(columns) / charWidth) - 6)
        // Rebuild only when the content changes, so a redraw does not reset
        // the scroll position; a new thread starts at the top.
        let key = "\(blocks.hashValue)-\(TextSize.shared.size)-\(columns)-\(nameChars)"
        if coordinator.key == key { return }
        let sameThread = coordinator.threadId == threadId
        coordinator.key = key
        coordinator.threadId = threadId
        let (string, kinds) = attributed(blocks, columns: columns, nameChars: nameChars)
        text.textStorage?.setAttributedString(string)
        if !sameThread { text.scrollToBeginningOfDocument(nil) }
        measure(text, kinds: kinds)
        // UI scripts: the positions of the reader's links are measured
        // again just before a click, the reader may have moved since (the
        // status bar moved above it), the text not.
        if UIScript.isActive {
            UIScript.remeasureReader = { [weak text] in
                if let text { measure(text, kinds: kinds) }
            }
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator: NSObject, NSTextViewDelegate {
        var key = ""
        var threadId = -1
        weak var controller: InterfaceController?
        /// Rebuilds the text for the current width (set by `updateNSView`).
        var rebuild: (() -> Void)?
        private var observer: NSObjectProtocol?

        func observe(_ view: NSView) {
            observer = NotificationCenter.default.addObserver(
                forName: NSView.frameDidChangeNotification, object: view, queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated { self?.rebuild?() }
            }
        }

        deinit {
            if let observer { NotificationCenter.default.removeObserver(observer) }
        }

        /// Header and "···" links go to the controller; other links (URLs
        /// in the mail) open normally.
        func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
            guard let url = link as? URL ?? (link as? String).flatMap(URL.init(string:)) else { return false }
            return MainActor.assumeIsolated { controller?.readerLink(url) ?? false }
        }
    }

    /// Kind of each block, with its character range, for `S` and tests.
    enum Region { case quote, text }

    private func attributed(_ blocks: [ThreadDisplay.Block], columns: Int, nameChars: Int)
        -> (NSAttributedString, [(Region, NSRange)])
    {
        let out = NSMutableAttributedString()
        var regions: [(Region, NSRange)] = []
        let paragraph = NSMutableParagraphStyle()
        // The body in the reading font of the theme, with its line spacing
        // (`fonts.reading`, `reading_line_spacing`); the lines around it
        // (headers, folded mails) in the interface font.
        paragraph.lineSpacing = Fonts.mailLineSpacing
        let muted = NSColor(Palette.muted)
        let body: [NSAttributedString.Key: Any] = [.font: Fonts.mailBody(14), .foregroundColor: NSColor(Palette.text), .paragraphStyle: paragraph]
        let dim: [NSAttributedString.Key: Any] = [.font: Fonts.mailBody(14), .foregroundColor: muted, .paragraphStyle: paragraph]
        let small: [NSAttributedString.Key: Any] = [.font: Fonts.scaled(12), .foregroundColor: muted]
        func add(_ s: String, _ attrs: [NSAttributedString.Key: Any], link: String? = nil, region: Region? = nil) {
            var a = attrs
            if let link, let url = URL(string: link) { a[.link] = url }
            let start = out.length
            out.append(NSAttributedString(string: s, attributes: a))
            if let region { regions.append((region, NSRange(location: start, length: out.length - start))) }
        }
        for b in blocks {
            switch b {
            case .separator:
                add("\n", small)
            case .header(let id, let folded, let title, let detail, let files, let address, let alert, let foldable, let book):
                let titleAttrs: [NSAttributedString.Key: Any] = [
                    .font: Fonts.scaled(12, weight: folded ? .regular : .semibold),
                    .foregroundColor: folded ? muted : NSColor(Palette.text),
                ]
                // In a conversation, a click on the header folds or unfolds
                // the message; a single mail has nothing to fold.
                let fold = foldable ? "nm://fold/\(id)" : nil
                add(title, titleAttrs, link: fold, region: .text)
                if let address {
                    // "· alice@atelier.fr": the address in full, a link
                    // (click: copied; ⌘-click: the list filtered on it), in
                    // the alert colour when it deserves a second look.
                    let addressAttrs: [NSAttributedString.Key: Any] = [
                        .font: Fonts.scaled(12), .foregroundColor: alert ? NSColor(Palette.error) : muted,
                    ]
                    add(" · ", small)
                    add(address, addressAttrs, link: "nm://sender/\(id)")
                }
                if let book {
                    // At the end of the line: the sender is in the address
                    // book, or not. A click does what `ctrl-c` does (opens
                    // the card, or copies the address).
                    add("  ", small)
                    out.append(Self.symbol(book.symbol, size: scaled(11), color: muted, link: "nm://book/\(id)",
                                           label: book.label))
                }
                if folded && files > 0 {
                    // A paperclip and the number of files of the folded mail.
                    add("  ", small)
                    out.append(Self.symbol("paperclip", size: scaled(11), color: muted, link: "nm://fold/\(id)"))
                    add("\(files)", small, link: fold)
                }
                if folded, let detail, !detail.isEmpty { add("  " + detail, small, link: fold) }
                add("\n", small)
                if !folded, let detail { add(detail + "\n", small) }
                if !folded { add("\n", small) }
            case .segment(_, _, let kind, let text, let depth):
                switch kind {
                case "text", "forward":
                    add(text + "\n\n", body, region: .text)
                default:
                    // Quotes (and revealed hidden parts) dimmed, with one bar
                    // per quote level, as in Apple Mail.
                    let bars = String(repeating: "│ ", count: max(1, depth))
                    let lines = text.split(separator: "\n", omittingEmptySubsequences: false).map { bars + $0 }
                    add(lines.joined(separator: "\n") + "\n\n", dim, region: .quote)
                }
            case .hidden(let id, let index, let label):
                // The link covers the label only, not the line breaks: a
                // click in the empty space after the line does nothing.
                add(label, small, link: "nm://reveal/\(id)/\(index)", region: .quote)
                add("\n\n", small)
            case .note(_, let text):
                add(text + "\n\n", small)
            case .attachments(_, let files):
                out.append(Self.tiles(files, columns: columns, nameChars: nameChars))
                add("\n", small)
            }
        }
        return (out, regions)
    }

    /// An SF Symbol as a character of the text, in `color`, aligned on the
    /// text around it.
    /// `label`: the tooltip, and what VoiceOver reads.
    static func symbol(_ name: String, size: CGFloat, color: NSColor, link: String? = nil, label: String? = nil)
        -> NSAttributedString {
        let config = NSImage.SymbolConfiguration(pointSize: size, weight: .regular)
            .applying(NSImage.SymbolConfiguration(paletteColors: [color]))
        guard let image = NSImage(systemSymbolName: name, accessibilityDescription: label)?.withSymbolConfiguration(config)
        else { return NSAttributedString() }
        let attachment = NSTextAttachment()
        attachment.image = image
        attachment.bounds = CGRect(x: 0, y: -size * 0.2, width: image.size.width, height: image.size.height)
        let s = NSMutableAttributedString(attachment: attachment)
        if let link, let url = URL(string: link) {
            s.addAttribute(.link, value: url, range: NSRange(location: 0, length: s.length))
        }
        if let label { s.addAttribute(.toolTip, value: label, range: NSRange(location: 0, length: s.length)) }
        return s
    }

    /// The attachments of a message as a grid of tiles (`ui.md`, "Pièces
    /// jointes"): a text table of `columns` cells per row, each cell with
    /// the type's icon, the name (cut in the middle) and the size. The
    /// whole cell is a link to `nm://attachment/<id>`, handled by the
    /// controller: a click saves the file, ⌘-click saves and opens it.
    ///
    /// AppKit note: `NSTextTable` lays out paragraphs as table cells; each
    /// paragraph says which cell it belongs to through its paragraph style
    /// (`textBlocks`).
    static func tiles(_ files: [AttachmentView], columns: Int, nameChars: Int) -> NSAttributedString {
        let out = NSMutableAttributedString()
        let table = NSTextTable()
        table.numberOfColumns = columns
        table.layoutAlgorithm = .fixedLayoutAlgorithm
        table.hidesEmptyCells = true
        let muted = NSColor(Palette.muted)
        for (i, a) in files.enumerated() {
            let cell = NSTextTableBlock(table: table, startingRow: i / columns, rowSpan: 1,
                                        startingColumn: i % columns, columnSpan: 1)
            cell.setValue(100 / CGFloat(columns), type: .percentageValueType, for: .width)
            cell.setWidth(1, type: .absoluteValueType, for: .border)
            cell.setBorderColor(NSColor(Palette.separator))
            cell.setWidth(scaled(8), type: .absoluteValueType, for: .padding)
            cell.setWidth(scaled(3), type: .absoluteValueType, for: .margin)
            let style = NSMutableParagraphStyle()
            style.textBlocks = [cell]
            style.lineBreakMode = .byClipping
            let url = URL(string: "nm://attachment/\(a.id)")!
            let name = AttachmentDisplay.middleTruncated(AttachmentDisplay.name(a), max: nameChars)
            let start = out.length
            out.append(symbol(AttachmentDisplay.symbol(mimeType: a.mimeType, filename: a.filename),
                              size: scaled(13), color: muted))
            out.append(NSAttributedString(string: "  " + name + "\n", attributes: [
                .font: Fonts.scaled(12), .foregroundColor: NSColor(Palette.text),
            ]))
            out.append(NSAttributedString(string: AttachmentDisplay.size(a.size) + "\n", attributes: [
                .font: Fonts.scaled(11), .foregroundColor: muted,
            ]))
            let range = NSRange(location: start, length: out.length - start)
            out.addAttributes([.paragraphStyle: style, .link: url], range: range)
        }
        return out
    }

    /// After layout: positions of the blocks for `S`, and, in UI script
    /// mode, the frames of the links and the displayed text.
    private func measure(_ text: NSTextView, kinds: [(Region, NSRange)]) {
        guard let lm = text.layoutManager, let tc = text.textContainer else { return }
        lm.ensureLayout(for: tc)
        let origin = text.textContainerOrigin
        func rect(_ r: NSRange) -> NSRect {
            let g = lm.glyphRange(forCharacterRange: r, actualCharacterRange: nil)
            return lm.boundingRect(forGlyphRange: g, in: tc).offsetBy(dx: origin.x, dy: origin.y)
        }
        var quotes: [(Double, Double)] = []
        var texts: [Double] = []
        for (region, range) in kinds where range.length > 0 {
            let r = rect(range)
            switch region {
            case .quote: quotes.append((Double(r.minY), Double(r.maxY)))
            case .text: texts.append(Double(r.minY))
            }
        }
        controller.readerSkip = (quotes, texts)
        // The top of each message's header (its first fold link), for
        // ctrl-k / ctrl-j, which scroll there once the reader is laid out.
        var headers: [Int: Double] = [:]
        if let storage = text.textStorage {
            storage.enumerateAttribute(.link, in: NSRange(location: 0, length: storage.length)) { value, range, _ in
                guard let url = value as? URL, url.scheme == "nm", url.host == "fold",
                      let id = Int(url.lastPathComponent), headers[id] == nil else { return }
                headers[id] = Double(rect(range).minY)
            }
        }
        controller.readerHeaders = headers
        if let message = controller.pendingMessageScroll, headers[message] != nil {
            controller.pendingMessageScroll = nil
            controller.scrollToMessage(message)
        }
        guard UIScript.isActive, let storage = text.textStorage else { return }
        UIScript.readerText = storage.string
        var n = 0, files = 0, senders = 0, books = 0
        storage.enumerateAttribute(.link, in: NSRange(location: 0, length: storage.length)) { value, range, _ in
            guard let url = value as? URL, url.scheme == "nm" else { return }
            if url.host == "book" {
                // The address book's icon of an unfolded message.
                UIScript.frames["reader-book-\(books)"] = text.convert(rect(range), to: nil)
                books += 1
                return
            }
            if url.host == "sender" {
                // The sender's address of an unfolded message.
                UIScript.frames["reader-sender-\(senders)"] = text.convert(rect(range), to: nil)
                senders += 1
                return
            }
            if url.host == "attachment" {
                // The first line of the tile (icon and name).
                let line = (storage.string as NSString).lineRange(for: NSRange(location: range.location, length: 0))
                UIScript.frames["attachment-\(files)"] = text.convert(rect(NSIntersectionRange(line, range)), to: nil)
                files += 1
                return
            }
            UIScript.frames["reader-link-\(n)"] = text.convert(rect(range), to: nil)
            n += 1
        }
        UIScript.readerLinks = n
        UIScript.readerAttachments = files
    }
}

/// HTML reader: a web view without JavaScript; links open in the browser.
/// Remote images come only through the app's `nm-remote:` scheme
/// (`RemoteImages.swift`), and only those of `allowedImages`.
private struct HTMLReader: NSViewRepresentable {
    let html: String
    /// Original URLs of the remote images the page may load.
    let allowedImages: Set<String>
    /// Attachment ids of the embedded images the page shows (`nm-cid:`).
    let inlineImages: Set<Int>
    /// Whether the page runs Dark Reader (`DarkMode`).
    let darkMode: Bool
    /// Keeps the web view, to scroll it from the keyboard (Space, ctrl-f…).
    let controller: InterfaceController

    func makeNSView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        // The HTML is already sanitized by the core (no script); turning
        // JavaScript off is a second barrier.
        config.defaultWebpagePreferences.allowsContentJavaScript = false
        // A scheme handler can only be set before the web view is created.
        config.setURLSchemeHandler(context.coordinator.images, forURLScheme: RemoteImages.scheme)
        config.setURLSchemeHandler(context.coordinator.inline, forURLScheme: InlineImages.scheme)
        // Nothing in the page's own storage outlives the page.
        config.websiteDataStore = .nonPersistent()
        let view = WKWebView(frame: .zero, configuration: config)
        view.navigationDelegate = context.coordinator
        view.setValue(false, forKey: "drawsBackground")
        controller.readerWeb = view
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {
        let c = context.coordinator
        guard c.html != html || c.images.allowed != allowedImages || c.inline.allowed != inlineImages else { return }
        // Scripts of the app apply from the next load on: set before it.
        let scripts = view.configuration.userContentController
        scripts.removeAllUserScripts()
        if darkMode, let script = DarkMode.userScript() { scripts.addUserScript(script) }
        c.html = html
        c.images.allowed = allowedImages
        c.inline.allowed = inlineImages
        // The page is loaded once the block list is in place (compiled
        // once, then immediate). A newer page asked meanwhile wins.
        Task { @MainActor in
            if let list = await RemoteBlocker.list(), !c.blocking {
                view.configuration.userContentController.add(list)
                c.blocking = true
            }
            guard c.html == html else { return }
            if UIScript.isActive { UIScript.remoteImagesAllowed = allowedImages.count }
            view.loadHTMLString(html, baseURL: nil)
        }
    }

    func makeCoordinator() -> Coordinator {
        let c = Coordinator()
        c.controller = controller
        // Swift note: `[weak model]` keeps the closure from holding the
        // model alive; it lives as long as the web view.
        let model = controller.model
        c.inline.load = { [weak model] id in await model?.inlineImage(id) }
        return c
    }

    final class Coordinator: NSObject, WKNavigationDelegate {
        var html = ""
        weak var controller: InterfaceController?
        /// The handler of `nm-remote:` sources; kept by the coordinator,
        /// which lives as long as the web view.
        let images = MainActor.assumeIsolated { RemoteImageLoader() }
        /// The handler of `nm-cid:` sources (embedded images).
        let inline = MainActor.assumeIsolated { InlineImageLoader() }
        /// Whether the block list was added to the web view.
        var blocking = false

        /// In UI script mode, reports what the page shows, since a snapshot
        /// of the window cannot see a web view (it draws in another
        /// process): its text, whether the bundled font loaded, the font
        /// family applied to the body, the embedded images drawn, the font
        /// of the first `<font face>`, the width of the first table with a
        /// width, and whether the Content Security Policy is in the page.
        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            // ctrl-k / ctrl-j: the message to show, once the page is there.
            MainActor.assumeIsolated {
                if let message = controller?.pendingMessageScroll {
                    controller?.pendingMessageScroll = nil
                    controller?.scrollToMessage(message)
                }
            }
            guard UIScript.isActive else { return }
            // The messages are in shadow roots (`MailFrame`): each query
            // looks in the page and in every frame.
            let js = """
            (() => {
            const frames = [...document.querySelectorAll('.nm-msg')].map(h => h.shadowRoot).filter(r => r);
            const all = s => [document, ...frames].flatMap(r => [...r.querySelectorAll(s)]);
            const first = s => all(s)[0];
            const bodies = frames.map(r => r.querySelector('.nm-body')).filter(b => b);
            return JSON.stringify({
                text: [document.body.innerText, ...bodies.map(b => b.innerText)].join('\\n').slice(0, 2000),
                font: document.fonts.check('14px "NM Mono"'),
                family: getComputedStyle(document.body).fontFamily,
                inline: all('img[src^="nm-cid:"]').filter(i => i.complete && i.naturalWidth > 0).length,
                authorFont: (f => f ? getComputedStyle(f).fontFamily : '')(first('font[face]')),
                tableWidth: (t => t ? t.offsetWidth : '')(first('table[width]')),
                csp: !!document.querySelector('meta[http-equiv="Content-Security-Policy"]'),
                frames: frames.length,
                paper: document.querySelectorAll('.nm-msg.paper').length,
                styledWidth: (e => e ? e.offsetWidth : '')(first('.probe-width')),
                styledBackground: (e => e ? getComputedStyle(e).backgroundColor : '')(first('.probe-background')),
            });
            })()
            """
            webView.evaluateJavaScript(js) { result, _ in
                guard let json = result as? String, let data = json.data(using: .utf8),
                      let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
                MainActor.assumeIsolated {
                    UIScript.html = obj.mapValues { "\($0)" }
                }
            }
            // Dark Reader, in its own world: whether it is on, and the
            // requests it asked for (refused).
            webView.evaluateJavaScript(
                "typeof DarkReader === 'undefined' ? 'absent' : (DarkReader.isEnabled() ? 'on' : 'off') + ' ' + window.__nmDarkFetches",
                in: nil, in: DarkMode.world
            ) { result in
                let value = (try? result.get()) as? String ?? "absent"
                MainActor.assumeIsolated {
                    let parts = value.split(separator: " ")
                    UIScript.darkReader = parts.first.map(String.init) ?? "absent"
                    UIScript.darkReaderFetches = parts.count > 1 ? String(parts[1]) : ""
                }
            }
        }
        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                     decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
            // An attachment tile: saved, and opened too with ⌘.
            if let url = action.request.url, url.scheme == "nm", url.host == "attachment",
               let id = Int(url.lastPathComponent) {
                let open = action.modifierFlags.contains(.command)
                MainActor.assumeIsolated { controller?.model.saveAttachment(id, open ? .saveAndOpen : .save) }
                decisionHandler(.cancel)
                return
            }
            // The sender's address: copied, or the list filtered on it.
            if let url = action.request.url, url.scheme == "nm", url.host == "sender" {
                let command = action.modifierFlags.contains(.command)
                MainActor.assumeIsolated { _ = controller?.readerLink(url, command: command) }
                decisionHandler(.cancel)
                return
            }
            // Only the document itself loads here; a clicked link opens in
            // the default browser, a `tel:` link calls (`AppModel.openLink`,
            // which may confirm the call too).
            if action.navigationType == .linkActivated, let url = action.request.url {
                MainActor.assumeIsolated {
                    if let model = controller?.model { model.openLink(url) } else { NSWorkspace.shared.open(url) }
                }
                decisionHandler(.cancel)
            } else {
                decisionHandler(.allow)
            }
        }
    }
}

// MARK: - Menus and pickers

/// Helix-style key menu, in the bottom-right corner: one line per key.
/// The name of a virtual folder (Work, All Mail, Everything) in the menu
/// `;` and the folder column (`ui.md`, "Dossiers virtuels"): a stack icon
/// before it and the accent colour, so that it reads as a gathering of
/// folders, not as one more folder. Neither an italic (the monospace font
/// of the geek theme has none) nor a block of its own: each ends the block
/// of the folders it gathers.
/// The unread count of a folder as a small pill tinted with the unread
/// color, as the counts of Mail's sidebar: in the menu `;` and the folder
/// column when the keys are on the right (`KeyMenu.detailAsBadge`), where a
/// bare number would be read with the key beside it ("18 i").
struct UnreadBadge: View {
    let count: String
    /// On a row highlighted in the accent (regular): the pill takes the
    /// text color of the selection, or it would vanish in the blue.
    var onAccent = false

    var body: some View {
        let ink = onAccent ? Palette.selectionText : Palette.unread
        Text(count)
            .font(.interface(11, weight: .semibold))
            .monospacedDigit()
            .foregroundStyle(ink)
            .padding(.horizontal, scaled(6))
            .padding(.vertical, scaled(1))
            .background(Capsule().fill(ink.opacity(0.18)))
    }
}

struct VirtualFolderLabel: View {
    let name: String
    /// Its color: the accent, or the text color of the selection on a row
    /// highlighted in the accent (`KeyMenu.onAccent`).
    var color: Color = Palette.accentText

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: "square.stack").imageScale(.small)
            Text(name)
        }
        .foregroundStyle(color)
    }
}

/// The panel of a choice (`InterfaceController.activeChoice`): its title and
/// one row per answer, each with its key. Drawn in the bottom-right
/// corner, or by the screen itself where the choice belongs.
struct ChoicePanel: View {
    let controller: InterfaceController
    let choice: InterfaceController.Choice

    var body: some View {
        KeyMenu(title: controller.choiceTitle(choice),
                entries: controller.choiceEntries(choice).map { .init(key: $0.key, label: $0.label, gapBefore: $0.gapBefore, detail: $0.detail) },
                highlighted: controller.choiceIndex,
                onSelect: { controller.chooseChoiceEntry($0) })
    }
}

struct KeyMenu: View {
    let title: String
    let entries: [Row]
    /// Row highlighted with the arrows, if any.
    var highlighted: Int? = nil

    /// One line: the key, the label, grayed out or not, a gap above (start
    /// of a block), and a detail aligned on the right (unread count).
    struct Row {
        let key: String
        let label: String
        var muted = false
        var gapBefore = false
        var detail: String? = nil
        /// An information ("T: and open") rather than a count to notice.
        var detailMuted = false
        /// A virtual folder of the menu `;` (`VirtualFolderMark`).
        var virtual = false
    }
    /// Called with the row clicked; `nil` for a menu that only informs
    /// (the help).
    let onSelect: ((Int) -> Void)?

    /// Whether a row's detail is drawn as a badge: a count (not an
    /// information, `detailMuted`) next to a key on the right
    /// (`layout.menu_keys = "right"`: clean, regular). With the key first
    /// (geek), the count is alone at the right end and reads as it is.
    static func detailAsBadge(keysRight: Bool, muted: Bool) -> Bool { keysRight && !muted }

    /// Whether row `index` is drawn on a fill of the accent: highlighted
    /// with the arrows, in a theme whose selection has a text color of its
    /// own (`Look.strongSelection`: white on the accent in regular). Its
    /// texts then take that color; elsewhere (geek, clean) the highlight
    /// is pale and they keep theirs.
    static func onAccent(_ index: Int, highlighted: Int?, strongSelection: Bool) -> Bool {
        strongSelection && index == highlighted
    }

    /// Room added before the key of a row that has a detail, beyond the
    /// spacing of the row (10 pt), at a 12 pt text.
    static let keyGapAfterBadge: CGFloat = 8

    var body: some View {
        // Width of the key column: the longest key, measured in the
        // interface font (in characters, in a font of fixed pitch).
        let measure = TextMeasure.interface(12 * TextSize.shared.scale)
        let keyWidth = (entries.map { measure.width($0.key) }.max() ?? measure.char) + 4
        VStack(alignment: .leading, spacing: 2) {
            // The title: in geek, a line in the accent; in a theme of
            // rounded panels, the heading of a section of a macOS menu,
            // small, grey, in capitals, over a line that sets it apart
            // from the rows (`Look.roundedPanels`).
            if Look.roundedPanels {
                Text(title.uppercased(with: uiLocale))
                    .font(.interface(10, weight: .semibold))
                    .tracking(0.6)
                    .foregroundStyle(Palette.muted)
                    .padding(.horizontal, Look.menuRowTextInset)
                    .padding(.top, 2)
                Palette.separator.frame(height: 1)
                    .padding(.horizontal, Look.menuRowTextInset)
                    .padding(.top, 4)
                    .padding(.bottom, 4)
            } else {
                Text(title).foregroundStyle(Palette.accentText).padding(.bottom, 2)
            }
            ForEach(Array(entries.enumerated()), id: \.offset) { i, e in
                // The row highlighted in a fill of the accent (regular): its
                // label, key and count take the text color of the selection,
                // white, or they would be lost in the blue.
                let accent = Self.onAccent(i, highlighted: highlighted, strongSelection: Look.strongSelection)
                HStack(spacing: 10) {
                    // The key first, as in Helix, or last, as in a macOS
                    // menu (`layout.menu_keys`).
                    if !Look.menuKeysRight {
                        Text(e.key).foregroundStyle(accent ? Palette.selectionText : Palette.accentText)
                            .frame(width: keyWidth, alignment: .leading)
                    }
                    if e.virtual {
                        VirtualFolderLabel(name: e.label, color: accent ? Palette.selectionText : Palette.accentText)
                    } else if accent {
                        // A row not available yet stays fainter than the others.
                        Text(e.label).foregroundStyle(Palette.selectionText.opacity(e.muted ? 0.7 : 1))
                    } else {
                        Text(e.label).foregroundStyle(e.muted ? Palette.muted : Palette.text)
                    }
                    // Pushes the detail to the right end; at least a gap
                    // of two characters after the longest label.
                    Spacer(minLength: scaled(16))
                    if let detail = e.detail {
                        if Self.detailAsBadge(keysRight: Look.menuKeysRight, muted: e.detailMuted) {
                            // Beside a key on the right, a bare count read
                            // with it ("18 i"): a small pill tinted with the
                            // unread color, as the counts of Mail's sidebar,
                            // tells it apart from the key at a glance.
                            UnreadBadge(count: detail, onAccent: accent)
                                .uiTarget("menu-detail-\(i)")
                        } else if accent {
                            Text(detail).foregroundStyle(Palette.selectionText.opacity(e.detailMuted ? 0.8 : 1))
                        } else {
                            Text(detail).foregroundStyle(e.detailMuted ? Palette.muted : Palette.unread)
                        }
                    }
                    if Look.menuKeysRight {
                        // More room before the key than between the other
                        // parts of the row: the count and the key are two
                        // things, not one.
                        // Grey as a shortcut of a macOS menu; on the accent,
                        // the selection's text color, a little fainter.
                        Text(e.key).foregroundStyle(accent ? Palette.selectionText.opacity(0.8) : Palette.muted)
                            .frame(width: keyWidth, alignment: .trailing)
                            .uiTarget("menu-key-\(i)")
                            .padding(.leading, e.detail == nil ? 0 : scaled(Self.keyGapAfterBadge))
                    }
                }
                // Every row takes the menu's width (that of its widest
                // row), so the details line up on the right edge.
                .frame(maxWidth: .infinity)
                // The theme's height of a row (`layout.menu_row_height`);
                // that of its text in geek.
                .frame(height: Look.menuRowHeight.map(scaled))
                // The text inside the highlighted row, with room before
                // the label and after the key; the highlight across the
                // panel (`Look.menuRowTextInset`).
                .padding(.horizontal, Look.menuRowTextInset)
                .rowFill(i == highlighted ? Palette.cursor : nil, inset: 0)
                .contentShape(Rectangle())
                .onTapGesture { onSelect?(i) }
                .uiTarget("menu-\(i)")
                // A blank line between blocks (folders to deal with, filed,
                // sent, thrown away; screens).
                .padding(.top, e.gapBefore ? scaled(10) : 0)
            }
        }
        // As wide as its longest row, not as the window: the rows' spacers
        // (which make the whole row clickable) would stretch it otherwise.
        .fixedSize(horizontal: true, vertical: false)
        // The highlight of a row runs close to the edges of a rounded
        // panel, as in a macOS menu; its text keeps its room inside.
        .padding(.vertical, 10 + Look.popupExtra / 2)
        .padding(.horizontal, Look.roundedPanels ? 6 : 10)
        .popupChrome()
    }
}

/// The list of keys (`?`, `ui.md`): the groups of `Help.groups` side by
/// side in up to three columns, centered, so that every key is seen at
/// once, without scrolling. The window's width decides the number of
/// columns (`Help.columnCount`); `Help.columns` balances them.
private struct HelpPanel: View {
    let groups: [HelpGroup]

    /// Inner margin of the panel, and margin around it in the window.
    private var padding: CGFloat { scaled(14) + Look.popupExtra }
    private var margin: CGFloat { scaled(12) }

    var body: some View {
        let measure = TextMeasure.interface(12 * TextSize.shared.scale)
        GeometryReader { geometry in
            let count = Help.columnCount(groups, available: geometry.size.width - 2 * margin,
                                         measure: measure, padding: padding)
            let columns = Help.columns(groups, count: count)
            VStack(alignment: .leading, spacing: scaled(10)) {
                Text(L("Keys")).foregroundStyle(Palette.accentText)
                // Swift note: `ViewThatFits` shows the first of its views
                // that fits in the space offered. The columns alone almost
                // always fit; in a very short window they scroll instead of
                // being cut.
                ViewThatFits(in: .vertical) {
                    columnsView(columns, measure: measure)
                    ScrollView { columnsView(columns, measure: measure) }
                }
            }
            .fixedSize(horizontal: true, vertical: false)
            .padding(padding)
            .popupChrome()
            .padding(margin)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .onAppear { if UIScript.isActive { UIScript.helpColumns = columns.map { $0.map(\.title) } } }
            .onChange(of: count) { _, _ in
                if UIScript.isActive { UIScript.helpColumns = columns.map { $0.map(\.title) } }
            }
        }
    }

    private func columnsView(_ columns: [[HelpGroup]], measure: TextMeasure) -> some View {
        let charWidth = measure.char
        return HStack(alignment: .top, spacing: CGFloat(Help.columnGap) * charWidth) {
            ForEach(Array(columns.enumerated()), id: \.offset) { _, column in
                // One key width for the whole column: the labels of its
                // groups line up, as in a single table.
                let keyWidth = Help.keyWidth(column, measure) + 4
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(Array(column.enumerated()), id: \.offset) { g, group in
                        Text(group.title)
                            .bold()
                            .foregroundStyle(Palette.text)
                            .padding(.bottom, 2)
                            // A blank line between two groups.
                            .padding(.top, g == 0 ? 0 : scaled(10))
                        ForEach(Array(group.entries.enumerated()), id: \.offset) { _, e in
                            HStack(spacing: CGFloat(Help.keyGap) * charWidth) {
                                Text(e.key).foregroundStyle(Palette.accentText)
                                    .frame(width: keyWidth, alignment: .leading)
                                Text(e.label).foregroundStyle(e.notYet ? Palette.muted : Palette.text)
                                    .lineLimit(1)
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Helix-style picker: the typed text on top, the filtered rows below.
struct PickerPopup: View {
    let controller: InterfaceController
    let kind: InterfaceController.PickerKind
    /// Width of the panel (`width(_:available:scale:)`).
    let width: CGFloat
    /// The height of the window's content: a panel never goes past it.
    var available: CGFloat = .infinity

    /// Width of a picker panel, in points, for a window content `available`
    /// points wide at the text size `scale`:
    ///
    /// - the links panel (`l`) as wide as the window allows, with a margin
    ///   on each side, up to 1,100 points at the default size: link texts
    ///   and addresses are long, and cut in the middle when they do not fit;
    /// - an attribute with a value, or the search, 680 points (`ui.md`);
    /// - any other picker 520 points;
    ///
    /// never wider than the window.
    static func width(_ kind: InterfaceController.PickerKind, available: CGFloat, scale: CGFloat) -> CGFloat {
        let wanted: CGFloat
        switch kind {
        case .links: wanted = max(min(available - 2 * 40 * scale, 1100 * scale), 520 * scale)
        // The AI palette as wide as the search: its result takes its place,
        // and a text rewritten reads better on long lines.
        case .attributeValue, .search, .ai: wanted = 680 * scale
        default: wanted = 520 * scale
        }
        return min(wanted, available)
    }

    /// The panel's height, fixed so that it does not jump while typing
    /// filters the rows: taller for the panels that show details. Shared
    /// with the AI result, which takes the palette's place.
    ///
    /// The attribute selector (`f`) is as tall as all its rows: its rows
    /// are few and fixed, and one that had to be scrolled to was missed
    /// (Mail date… and "clear" were below the edge until November 2026).
    static func height(_ kind: InterfaceController.PickerKind, scale: CGFloat) -> CGFloat {
        switch kind {
        case .attributes:
            (headerHeight + 1 + CGFloat(InterfaceController.attributeRowCount) * rowHeight
                + CGFloat(InterfaceController.attributeGapCount) * gapHeight + 4) * scale
        case .attributeValue, .search, .links: 440 * scale
        default: 320 * scale
        }
    }

    /// Height of the input line, at the default text size: 24 in geek, a
    /// roomier field in a theme of rounded panels (`Look.roundedPanels`),
    /// as the search field of a web page.
    static var headerHeight: CGFloat { Look.roundedPanels ? 42 : 24 }
    /// Text size of the input line: bigger in a theme of rounded panels.
    static var headerTextSize: CGFloat { Look.roundedPanels ? 15 : 12 }
    /// Text size of the rows under it: bigger too, a step under the line.
    static var rowTextSize: CGFloat { Look.roundedPanels ? 14 : 12 }
    /// 20, or the theme's (`layout.menu_row_height`).
    static var rowHeight: CGFloat { Look.menuRowHeight ?? 20 }
    /// The line between two groups of rows (`Entry.gapBefore`: the pairs
    /// of the attribute selector), with its space above and below.
    static let gapHeight: CGFloat = 7

    /// `height`, kept inside a window `available` points tall, with a
    /// margin above and below: in a short window, the rows scroll.
    static func height(_ kind: InterfaceController.PickerKind, scale: CGFloat, available: CGFloat) -> CGFloat {
        min(height(kind, scale: scale), max(available - 2 * 16 * scale, 120 * scale))
    }

    var body: some View {
        let entries = controller.pickerEntries(kind)
        let current = controller.pickerIndex
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Text(title).foregroundStyle(Palette.accentText)
                // The caret is drawn where the cursor is, and a selected
                // text (the current words of Subject…, Text…) is
                // highlighted instead: `CaretText` does both.
                CaretText(line: controller.pickerLine)
                // What can be typed, until something is (`pickerHint`).
                if controller.pickerQuery.isEmpty, let hint = controller.pickerHint(kind) {
                    Text(hint).foregroundStyle(Palette.muted).lineLimit(1).truncationMode(.tail)
                }
                Spacer()
                Text("\(entries.count)").foregroundStyle(Palette.muted)
            }
            .font(.interface(Self.headerTextSize))
            .padding(.horizontal, 8 + Look.popupExtra + (Look.roundedPanels ? 4 : 0))
            .frame(height: scaled(Self.headerHeight))
            .uiTarget("picker-header")
            Palette.separator.frame(height: 1)
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(entries.enumerated()), id: \.offset) { i, e in
                            // A thin line, discreet, between two groups of
                            // rows: the pairs of the attribute selector.
                            if e.gapBefore {
                                Palette.separator.opacity(0.6).frame(height: 1)
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, scaled((Self.gapHeight - 1) / 2))
                            }
                            // The row of the cursor on a fill of the accent
                            // (regular): its texts white (`Look.ink`).
                            let on = i == current
                            HStack(spacing: 8) {
                                Text(e.key).foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                                    .frame(width: scaled(22), alignment: .leading)
                                if let icon = e.icon {
                                    Image(systemName: icon).foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                                        .frame(width: scaled(16))
                                }
                                if halves {
                                    // Links: what the mail calls the link on the
                                    // left, where it goes on the right, each in
                                    // half of the row, so that a long text never
                                    // pushes the address out of sight. Two frames
                                    // equally flexible get the same width.
                                    Text(e.label).lineLimit(1).truncationMode(.middle)
                                        .foregroundStyle(Look.ink(Palette.text, selected: on))
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                    Text(e.detail ?? "").lineLimit(1).truncationMode(.middle)
                                        .foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                } else {
                                    Text(e.label).lineLimit(1).truncationMode(.middle)
                                        .foregroundStyle(Look.ink(Palette.text, selected: on))
                                    Spacer()
                                    // At the right end of the row: an unread count
                                    // stands out, an information stays muted.
                                    if let detail = e.detail {
                                        Text(detail).lineLimit(1)
                                            .foregroundStyle(e.detailStyle == .count
                                                ? Look.ink(Palette.unread, selected: on)
                                                : Look.ink(Palette.muted, selected: on, faint: true))
                                    }
                                }
                            }
                            .font(.interface(Self.rowTextSize))
                            .padding(.horizontal, 8 + Look.popupExtra)
                            .frame(height: scaled(Self.rowHeight))
                            .rowFill(i == current ? Palette.cursor : nil, inset: Look.popupExtra / 2)
                            .contentShape(Rectangle())
                            .onTapGesture { controller.clickPicker(kind, i) }
                            .uiTarget("picker-\(i)")
                            .id(i)
                        }
                    }
                }
                .scrollIndicators(Look.scrollIndicators)
                .onChange(of: controller.pickerIndex) { _, i in proxy.scrollTo(i) }
            }
            .overlay {
                if entries.isEmpty { Text(emptyText).foregroundStyle(Palette.muted) }
            }
        }
        // An attribute with a value gets a bigger panel at once (`ui.md`),
        // and so do the links of a mail.
        .frame(width: width, height: panelHeight)
        .onAppear {
            if UIScript.isActive {
                UIScript.pickerWidth = width
                UIScript.pickerVisibleRows = visibleRows
            }
        }
        .onChange(of: panelHeight) { _, _ in if UIScript.isActive { UIScript.pickerVisibleRows = visibleRows } }
        .onChange(of: width) { _, w in if UIScript.isActive { UIScript.pickerWidth = w } }
        .popupChrome()
    }

    private var panelHeight: CGFloat { Self.height(kind, scale: TextSize.shared.scale, available: available) }

    /// Rows that fit below the input line without scrolling (UI scripts).
    private var visibleRows: Int {
        let gaps = CGFloat(controller.pickerEntries(kind).filter(\.gapBefore).count) * scaled(Self.gapHeight)
        return Int((panelHeight - scaled(Self.headerHeight) - 1 - gaps) / scaled(Self.rowHeight))
    }

    /// Rows in two halves, text and address (the links of `l`).
    private var halves: Bool { kind == .links }

    private var title: String {
        switch kind {
        case .accounts: L("Account")
        case .attributes: L("Attributes")
        case .attributeValue(let field): field.title
        // The widest search: words anywhere, addresses included.
        case .search: L("Search everywhere")
        case .links: L("Links")
        case .tag: L("Tag")
        case .group: L("Group")
        case .ai: L("AI")
        case .rename(.tag, _, _): L("Rename the tag")
        case .rename(.group, _, _): L("Rename the group")
        case .themes: L("Theme")
        }
    }

    private var emptyText: String {
        switch kind {
        case .links: L("No link or attachment in this mail")
        case .tag, .group: L("Type a name")
        case .accounts: L("No account")
        case .attributes: L("No attribute")
        case .ai: L("No AI prompt matches")
        case .themes: L("No theme")
        case .rename: controller.pickerQuery.trimmingCharacters(in: .whitespaces).isEmpty ? L("Type a name") : ""
        // The value panels: the input line shows the hint while nothing is
        // typed; text that gives no row (a date that is none) shows it
        // here instead.
        case .attributeValue, .search: controller.pickerQuery.isEmpty ? "" : controller.pickerHint(kind) ?? ""
        }
    }
}

// MARK: - Screens

/// Generic full-window list with a title line.
private struct ScreenList<Row: View>: View {
    let title: String
    let hint: String
    let count: Int
    let index: Int
    /// Called with the row's position when it is clicked.
    let onClick: (Int) -> Void
    /// Called with the row's position when it comes into view: the wheel
    /// reaches the end of what is loaded without moving the cursor.
    var onRowAppear: (Int) -> Void = { _ in }
    /// Called with the height of the list: `ctrl-d` moves by half of what
    /// fits in it.
    var onHeight: (CGFloat) -> Void = { _ in }
    @ViewBuilder let row: (Int) -> Row

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(title).font(.interface(13, weight: .bold))
                Text(hint).foregroundStyle(Palette.muted)
                Spacer()
            }
            .padding(.horizontal, 10)
            .frame(height: scaled(26))
            Palette.separator.frame(height: 1)
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(0..<count, id: \.self) { i in
                            row(i)
                                .padding(.horizontal, 10)
                                .frame(height: Look.rowHeight)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .rowFill(i == index ? Palette.cursor : nil, index: i)
                                .contentShape(Rectangle())
                                .onTapGesture { onClick(i) }
                                .uiTarget("screen-\(i)")
                                .id(i)
                                .onAppear { onRowAppear(i) }
                        }
                    }
                }
                .scrollIndicators(Look.scrollIndicators)
                .onChange(of: index) { _, i in proxy.scrollTo(i) }
                .onGeometryChange(for: CGFloat.self, of: { $0.size.height }, action: onHeight)
            }
        }
    }
}

private struct ContactsScreen: View {
    let controller: InterfaceController

    var body: some View {
        ScreenList(
            title: L("Contacts"),
            hint: L("%@ filter · %@: decision · %@: domain decision · %@: human or robot · %@: their mails · esc: back",
                    controller.screenKey(.contactsFilter), controller.screenKey(.contactsDecideAddress),
                    controller.screenKey(.contactsDecideDomain), controller.screenKey(.contactsSenderKind),
                    controller.screenKey(.screenOpen)),
            count: controller.contacts.count, index: controller.screenIndex,
            onClick: { controller.clickScreenRow($0) },
            onRowAppear: { controller.screenRowAppeared($0) },
            onHeight: { controller.screenHeightChanged($0) }
        ) { i in
            let c = controller.contacts[i]
            // The row of the cursor on a fill of the accent (regular): its
            // texts white (`Look.ink`).
            let on = i == controller.screenIndex
            HStack(spacing: 8) {
                Text(c.addr).foregroundStyle(Look.ink(Palette.text, selected: on)).frame(width: scaled(300), alignment: .leading)
                Text(c.name ?? "").foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                    .frame(width: scaled(200), alignment: .leading)
                // The address's decision, or the domain's (marked "@"),
                // which applies when the address has none.
                if let own = c.screeningTarget {
                    Text(controller.model.folderNames.name(own)).foregroundStyle(Look.ink(Palette.text, selected: on))
                        .frame(width: scaled(110), alignment: .leading)
                } else {
                    Text(c.domainTarget.map { "@" + controller.model.folderNames.name($0) } ?? "")
                        .foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                        .frame(width: scaled(110), alignment: .leading)
                }
                // Human or robot; a choice of the user is marked "*".
                Text(c.originLabel).foregroundStyle(c.forced ? Look.ink(Palette.text, selected: on)
                                                             : Look.ink(Palette.muted, selected: on, faint: true))
                    .frame(width: scaled(80), alignment: .leading)
                Text("\(c.messageCount)").foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                    .frame(width: scaled(60), alignment: .trailing)
                Text(c.groups.map { "@" + $0 }.joined(separator: " ")).foregroundStyle(Look.ink(Palette.accentText, selected: on))
            }
            .lineLimit(1)
        }
    }
}

private struct HistoryScreen: View {
    let controller: InterfaceController

    var body: some View {
        // Read once: the rows are worked out from the groups and their lines.
        let rows = controller.historyRows
        let open = controller.screenKey(.screenOpen), read = controller.screenKey(.historyToggleRead)
        let who = controller.screenKey(.historyFilterSource)
        ScreenList(
            title: L("History"),
            hint: controller.historyIncludesRead
                ? L("%@: open or unfold · %@: hide read mails · %@: who did it · esc: back", open, read, who)
                : L("%@: open or unfold · %@: show read mails · %@: who did it · esc: back", open, read, who),
            count: rows.count, index: controller.screenIndex,
            onClick: { controller.clickScreenRow($0) },
            onRowAppear: { controller.screenRowAppeared($0) },
            onHeight: { controller.screenHeightChanged($0) }
        ) { i in
            HistoryRow(row: rows[i], names: controller.model.folderNames, selected: i == controller.screenIndex)
        }
    }
}

/// When, who, about what, the verb and what qualifies it. The subject takes
/// the room left: it is what tells two rows apart.
private struct HistoryRow: View {
    let row: HistoryDisplay.Row
    /// The names of the folders a move went to (`FolderNames`).
    let names: FolderNames
    /// The row of the cursor: on a fill of the accent (regular), its texts
    /// white (`Look.ink`).
    var selected = false

    var body: some View {
        let g = row.group
        let action = HistoryDisplay.action(g, names: names)
        let text = Look.ink(Palette.text, selected: selected)
        let muted = Look.ink(Palette.muted, selected: selected, faint: true)
        HStack(spacing: 8) {
            Text(Format.historyDate(g.date)).foregroundStyle(muted).frame(width: scaled(112), alignment: .leading)
            Text(HistoryDisplay.who(g.source)).foregroundStyle(Look.ink(Palette.accentText, selected: selected))
                .frame(width: scaled(84), alignment: .leading)
            HStack(spacing: scaled(5)) {
                if g.count > 1 {
                    Text(verbatim: row.isUnfolded ? "▾" : "▸").foregroundStyle(muted)
                }
                Text(HistoryDisplay.title(g)).foregroundStyle(text).truncationMode(.tail)
                if g.count > 1 {
                    Text(verbatim: "+\(g.count - 1)").foregroundStyle(muted).fixedSize()
                }
            }
            .padding(.leading, row.isDetail ? scaled(16) : 0)
            .frame(maxWidth: .infinity, alignment: .leading)
            Text(action.verb).foregroundStyle(text).frame(width: scaled(104), alignment: .leading)
            Text(action.detail).foregroundStyle(muted).frame(width: scaled(190), alignment: .leading)
        }
        .lineLimit(1)
    }
}

/// Tags or groups: a name and a count per row.
private struct NamesScreen: View {
    let controller: InterfaceController
    let title: String
    let rows: [(String, String)]

    var body: some View {
        ScreenList(
            title: title,
            hint: L("%@: their mails · %@: rename · %@: delete · esc: back", controller.screenKey(.screenOpen),
                    controller.screenKey(.labelRename), controller.screenKey(.labelDelete)), count: rows.count, index: controller.screenIndex,
            onClick: { controller.clickScreenRow($0) },
            onHeight: { controller.screenHeightChanged($0) }
        ) { i in
            // The row of the cursor on a fill of the accent: white.
            let on = i == controller.screenIndex
            HStack(spacing: 8) {
                Text(rows[i].0).foregroundStyle(Look.ink(Palette.text, selected: on)).frame(width: scaled(260), alignment: .leading)
                Text(rows[i].1).foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
            }
        }
    }
}
