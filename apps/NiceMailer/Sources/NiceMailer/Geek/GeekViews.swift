import AppKit
import SwiftUI
import WebKit

// Views of the geek theme (`ui.md`): an IDE-like window without rounded
// zones, scroll bars or animations; a compact status bar; Helix-style
// menus and pickers.

/// Root of the geek theme. Installs the keyboard monitor and lays out the
/// columns, the status bar and the overlays.
struct GeekRootView: View {
    @Environment(AppModel.self) private var model
    @State private var controller: GeekController?
    /// The AppKit event monitor, removed when the view goes away.
    @State private var monitor: Any?

    var body: some View {
        Group {
            if let controller {
                GeekWindow(controller: controller)
            } else {
                Palette.background
            }
        }
        .onAppear {
            let c = GeekController(model: model)
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
            // app before the views do. Returning nil swallows the event;
            // returning it lets AppKit go on (menus, ⌘ shortcuts).
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
                c.handle(event) ? nil : event
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
private struct GeekWindow: View {
    @Environment(AppModel.self) private var model
    let controller: GeekController

    var body: some View {
        // Rebuilt from scratch when the text size changes (`.id`): every
        // size and width of the theme derives from it, and rebuilding is
        // more reliable than counting on lazy rows to notice the change.
        let textSize = TextSize.shared.size
        VStack(spacing: 0) {
            if controller.statusPosition == .top { GeekStatusBar(controller: controller) }
            ZStack(alignment: .bottomTrailing) {
                content
                clickCatcher
                overlay
            }
            if controller.statusPosition == .bottom { GeekStatusBar(controller: controller) }
        }
        .background(Palette.background)
        .foregroundStyle(Palette.text)
        .font(.geek())
        // `ui.md`: no millisecond lost. Turning animations off for the
        // whole tree also removes SwiftUI's default fades.
        .transaction { $0.animation = nil; $0.disablesAnimations = true }
        .id(textSize)
    }

    @ViewBuilder private var content: some View {
        if case .upgrading(let message) = model.status {
            // The core upgrades the mailbox: say so, the app waits.
            UpgradingView(message: message).font(.geek(12))
        } else if case .failed(let message) = model.status, !UIScript.isActive {
            // No core: what to do, and the button that allows it.
            ConnectionErrorView(message: message).font(.geek(12))
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
                rows: controller.screenTags.map { ($0.name, $0.messageCount == 1 ? L("1 mail") : L("%lld mails", $0.messageCount)) })
        case .groups:
            NamesScreen(
                controller: controller, title: L("Groups"),
                rows: controller.groups.map { ($0.name, $0.memberCount == 1 ? L("1 member") : L("%lld members", $0.memberCount)) })
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
                    .background(Palette.popup)
                    .overlay(Rectangle().stroke(Palette.separator, lineWidth: 1))
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
    private func drawnInPlace(_ choice: GeekController.Choice) -> Bool {
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
                menu
                    .padding(.leading, max(0, x - 6))
                    .padding(.vertical, 2)
                    .frame(maxWidth: .infinity, maxHeight: .infinity,
                           alignment: controller.statusPosition == .bottom ? .bottomLeading : .topLeading)
            } else {
                // No block to drop from (status bar hidden, or another
                // screen): bottom-right corner, as in Helix.
                menu.padding(12)
            }
        case .help:
            HelpPanel(groups: Help.groups)
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

    private func menuTitle(_ p: GeekController.Prefix) -> String {
        switch p {
        case .move: L("Move")
        case .goto: L("Go to")
        case .save: L("Save")
        case .view: L("Display")
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
struct GeekStatusBar: View {
    @Environment(AppModel.self) private var model
    let controller: GeekController

    var body: some View {
        HStack(spacing: 10) {
            if controller.showsAppIcon, let icon = AppIcon.statusImage {
                // The normal mode: the app's icon, as tall as the text.
                Image(nsImage: icon)
                    .resizable()
                    .interpolation(.high)
                    .frame(width: scaled(15), height: scaled(15))
                    .accessibilityLabel(L("NiceMailer, normal mode"))
            } else {
                Text(" \(controller.modeLabel) ")
                    .font(.geek(11, weight: .bold))
                    .foregroundStyle(Palette.background)
                    .background(Palette.accentText)
            }
            let listsMail = controller.screen == .mail && (model.listFilter != nil || !model.listedFolderIds.isEmpty)
            if listsMail, model.listFilter == nil {
                // Account ("all" for the unified view) and folder, each with
                // its icon and the key that opens its picker; a click opens
                // it too.
                StatusBlock(icon: "at", text: model.scopeName, key: "@")
                    .onTapGesture { controller.openPickerFromStatusBar(.accounts) }
                    .uiTarget("status-account")
                StatusBlock(icon: GeekController.Prefix.goto.icon, text: model.currentFolderName, key: ";")
                    .menuAnchor(.goto, controller)
                    .onTapGesture { controller.openMenuFromStatusBar(.goto) }
                    .uiTarget("status-folder")
            } else {
                // A tag, group or contact list ("tag: Clients"), or a screen.
                Text(controller.locationLabel)
                    .lineLimit(1)
            }
            if listsMail {
                // Attributes narrowing the list, also in a tag, group or
                // contact list ("unread mails of the group"): the magnifier
                // opens the selector (`F`); each active attribute is a chip,
                // removed by a click.
                StatusBlock(icon: "magnifyingglass", text: "", key: "f")
                    .onTapGesture { controller.openPickerFromStatusBar(.attributes) }
                    .uiTarget("status-attributes")
                ForEach(model.attributes.chips) { chip in
                    Text(chip.label + " ×")
                        .lineLimit(1)
                        .foregroundStyle(Palette.accentText)
                        .padding(.horizontal, scaled(4))
                        .overlay(Rectangle().stroke(Palette.accent.opacity(0.5), lineWidth: 1))
                        .contentShape(Rectangle())
                        .onTapGesture { model.attributes.remove(chip.id) }
                        .uiTarget("chip-\(chip.id)")
                }
            }
            if controller.screen == .mail {
                // The other prefix menus, an icon and the key; the menu drops
                // down from the block. Three groups, set apart by a wide gap:
                // what the list shows (@ ; F, above), the display preference
                // (v), and actions on the current mail (m s).
                // In Quarantine, the `m` and `s` blocks go: a mail there can
                // only be moved (or flagged), and its targets are shown at
                // the right end of the bar instead (`quarantineTargets`).
                let prefixes: [GeekController.Prefix] = controller.directTargets.isEmpty ? [.view, .move, .save] : [.view]
                ForEach(prefixes, id: \.self) { p in
                    StatusBlock(icon: p.icon, text: "", key: p.rawValue)
                        .menuAnchor(p, controller)
                        .onTapGesture { controller.openMenuFromStatusBar(p) }
                        .uiTarget("status-\(p.rawValue)")
                        .padding(.leading, p == .save ? 0 : scaled(28))
                }
                // The AI, with the actions on the current mail: its palette
                // works on the mail (or the reply) as `m` and `s` do. In
                // Quarantine, where `m` and `s` go, it stays (sorting the
                // folder is its first prompt) and stands alone after `v`.
                StatusBlock(icon: "sparkles", text: "",
                            key: GeekController.aiKeyLabel(composing: model.composer != nil))
                    .onTapGesture { controller.openAIFromStatusBar() }
                    .help(L("AI assistance"))
                    .accessibilityLabel(L("AI assistance"))
                    .uiTarget("status-ai")
                    .padding(.leading, controller.directTargets.isEmpty ? 0 : scaled(28))
            }
            if let query = controller.contactsFilterChip {
                // The contacts filter, kept after Return: a chip like the
                // attributes of the mail list, removed by a click.
                Text("/" + query + " ×")
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .foregroundStyle(Palette.accentText)
                    .padding(.horizontal, scaled(4))
                    .overlay(Rectangle().stroke(Palette.accent.opacity(0.5), lineWidth: 1))
                    .contentShape(Rectangle())
                    .onTapGesture { controller.clearContactsFilter() }
                    .help(L("Contacts filtered on “%@”: click to show them all", query))
                    .accessibilityLabel(L("Contacts filtered on “%@”: click to show them all", query))
                    .uiTarget("chip-contacts")
            }
            if case .contactFilter = controller.overlay {
                HStack(spacing: 0) {
                    Text("/")
                    CaretText(line: controller.contactsLine)
                }
                .foregroundStyle(Palette.accentText)
            }
            Spacer(minLength: 8)
            if let error = model.lastError {
                Text(error).foregroundStyle(Palette.error).lineLimit(1)
            } else if let flash = model.flash {
                Text(flash).foregroundStyle(Palette.muted).lineLimit(1).truncationMode(.middle)
            } else if case .failed(let message) = model.status {
                Text(message).foregroundStyle(Palette.error).lineLimit(1)
            } else if case .upgrading = model.status {
                Text(L("upgrading the mailbox…")).foregroundStyle(Palette.muted).lineLimit(1)
            } else if let starting = model.startupPhase.message {
                Text(starting).foregroundStyle(Palette.muted).lineLimit(1)
            } else if let sort = model.aiSort {
                Text(AI.sortLine(sort)).foregroundStyle(Palette.accentText).lineLimit(1)
            } else if controller.screen == .mail, model.composer == nil, let s = model.selectedSuggestion {
                // Why the AI suggests a target for the mail under the cursor.
                Text(AI.suggestionLine(s)).foregroundStyle(Palette.accentText).lineLimit(1).truncationMode(.tail)
            }
            if !model.marked.isEmpty {
                Text(L("%lld marked", model.marked.count)).foregroundStyle(Palette.markedBar)
            }
            if model.sort == "modified" { Text(L("by modification")).foregroundStyle(Palette.muted) }
            quarantineTargets
            // The list of keys, last at the right end and on every screen:
            // a reminder that `?` exists, for whoever has not learned it
            // yet; a click opens it too.
            StatusBlock(icon: "questionmark.circle", text: "", key: "?")
                .onTapGesture { controller.toggleHelpFromStatusBar() }
                .help(L("Keys"))
                .uiTarget("status-help")
        }
        .font(.geek(11))
        .padding(.horizontal, 6)
        .frame(height: scaled(20))
        // Its empty parts move the window, as a title bar does: the title
        // bar is hidden by default (`WindowChrome`).
        .background(ZStack { Palette.statusBar; WindowDragArea() })
    }
}

extension GeekStatusBar {
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
            HStack(spacing: scaled(8)) {
                Image(systemName: GeekController.Prefix.move.icon)
                    .font(.system(size: scaled(10), weight: .semibold))
                    .foregroundStyle(Palette.muted)
                    .accessibilityLabel(L("Move"))
                ForEach(targets, id: \.self) { role in
                    StatusBlock(icon: nil, text: Role.name(role), key: Role.letter(role).lowercased())
                        .onTapGesture { controller.moveFromStatusBar(role) }
                        .uiTarget("status-target-\(role)")
                }
            }
            // A wide gap on each side: the targets are a group of their
            // own, apart from the messages and from `?`.
            .padding(.horizontal, scaled(18))
            // Never squeezed: a long message on their left is cut instead.
            .fixedSize(horizontal: true, vertical: false)
        }
    }
}

/// One element of the status bar: an SF Symbol, a name, and a reminder of
/// the key that opens it (`ui.md`, status bar).
extension View {
    /// Records the left edge of a status bar block, where the menu of
    /// `prefix` drops down from.
    fileprivate func menuAnchor(_ prefix: GeekController.Prefix, _ controller: GeekController) -> some View {
        background(GeometryReader { geo in
            Color.clear
                .onAppear { controller.menuBlockX[prefix] = geo.frame(in: .global).minX }
                .onChange(of: geo.frame(in: .global).minX) { _, x in controller.menuBlockX[prefix] = x }
        })
    }
}

private struct StatusBlock: View {
    /// `nil`: no icon (a move target of Quarantine, after the group's arrow).
    let icon: String?
    let text: String
    let key: String

    var body: some View {
        HStack(spacing: scaled(4)) {
            if let icon {
                Image(systemName: icon)
                    .font(.system(size: scaled(10), weight: .semibold))
                    .foregroundStyle(Palette.muted)
            }
            if !text.isEmpty { Text(text).lineLimit(1) }
            Text(key)
                .font(.geek(9))
                .foregroundStyle(Palette.muted)
                .padding(.horizontal, scaled(3))
                // Square corners: the geek theme has no rounded shapes (`ui.md`).
                .overlay(Rectangle().stroke(Palette.separator, lineWidth: 1))
        }
        .contentShape(Rectangle())
    }
}

// MARK: - Columns

/// 1, 2 or 3 columns (`ui.md`, `v1` `v2` `v3`), separated by 1-pixel lines;
/// the separator before the mail can be dragged.
private struct MailColumns: View {
    @Environment(AppModel.self) private var model
    let controller: GeekController

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
                    GeekReaderColumn(controller: controller)
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
            GeekFolderColumn(controller: controller).frame(width: folderWidth)
            Palette.separator.frame(width: 1)
        }
        GeekThreadColumn(controller: controller).frame(width: listWidth).clipped()
        if controller.columns > 1 {
            Palette.separator.frame(width: 1)
                // A wider invisible zone makes the line easy to grab.
                .overlay(Color.clear.frame(width: 7).contentShape(Rectangle()).onHover { inside in
                    if inside { NSCursor.resizeLeftRight.push() } else { NSCursor.pop() }
                })
                .gesture(DragGesture(minimumDistance: 1, coordinateSpace: .named("columns")).onChanged { g in
                    controller.listFraction = Double(max(0.15, min(0.7, (g.location.x - folderWidth) / max(rest, 1))))
                })
            GeekReaderColumn(controller: controller)
        }
    }
}

/// Third column: the folders of the current view, under a single title:
/// "all" (the unified view) or the account's name (`ui.md`).
private struct GeekFolderColumn: View {
    @Environment(AppModel.self) private var model
    let controller: GeekController

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Text(model.scopeName)
                    .foregroundStyle(Palette.muted)
                    .padding(.top, scaled(4)).padding(.bottom, scaled(2)).padding(.horizontal, 8)
                    .contentShape(Rectangle())
                    .onTapGesture { controller.openPickerFromStatusBar(.accounts) }
                let folders = controller.scopeFolders
                ForEach(Array(folders.enumerated()), id: \.element.id) { i, f in
                    HStack(spacing: 6) {
                        Text(f.letter.lowercased()).foregroundStyle(Palette.muted)
                        if f.virtual != nil {
                            VirtualFolderLabel(name: f.name)
                        } else {
                            Text(f.name)
                        }
                        Spacer()
                        if f.unread > 0 { Text("\(f.unread)").foregroundStyle(Palette.unread) }
                    }
                    .padding(.horizontal, 8)
                    .frame(height: scaled(20))
                    .background(controller.isCurrent(f) ? Palette.cursor : Color.clear)
                    // The whole row is clickable, not only its text.
                    .contentShape(Rectangle())
                    .onTapGesture { controller.open(f) }
                    .uiTarget("folder-\(f.name)")
                    // The same blocks as the ; menu.
                    .padding(.top, i > 0 && folders[i - 1].group != f.group ? scaled(10) : 0)
                }
                GeekColumnGroups(controller: controller)
            }
        }
        .scrollIndicators(.never)
    }
}

/// The contact groups, under the folders (`ui.md`, "Groupes dans la colonne
/// des dossiers"). A click on a group narrows the current list to the mails
/// of its members (the Group… attribute, chip in the status bar), a second
/// click removes it; ⌘-click excludes it. Several groups combine by "or".
/// With more groups than room, the most relevant ones are shown
/// (`GroupRanking`) and a last row "N more…" opens the Group… panel, which
/// lists them all. No groups: no block at all, not even its title.
private struct GeekColumnGroups: View {
    @Environment(AppModel.self) private var model
    let controller: GeekController

    var body: some View {
        let (rows, hidden) = controller.columnGroups
        if !rows.isEmpty {
            Text(L("Groups"))
                .foregroundStyle(Palette.muted)
                .padding(.top, scaled(14)).padding(.bottom, scaled(2)).padding(.horizontal, 8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
                .onTapGesture { controller.openGroupPanel() }
                .uiTarget("groups-title")
            ForEach(rows) { g in
                let choice = model.attributes.groupChoice(g.id)
                HStack(spacing: 6) {
                    // Where a folder has its letter: "•" when the group
                    // filters the list, "−" when it is excluded, as in the
                    // Group… panel.
                    Text(choice == true ? "•" : choice == false ? "−" : "@")
                        .foregroundStyle(choice == nil ? Palette.muted : Palette.accentText)
                    Text(g.name)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .foregroundStyle(choice == nil ? Palette.text : Palette.accentText)
                        .strikethrough(choice == false)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 8)
                .frame(height: scaled(20))
                // The same grey row as the current folder above.
                .background(controller.isHighlighted(g) ? Palette.cursor : Color.clear)
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
                // Two keys rather than plural variations: `L()` formats with
                // `String(format:)`, which does not apply them.
                Text(hidden == 1 ? L("1 more…") : L("%lld more…", hidden))
                    .foregroundStyle(Palette.muted)
                    .padding(.horizontal, 8)
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
private struct GeekThreadColumn: View {
    let controller: GeekController

    var body: some View {
        GeekThreadRows(controller: controller)
    }
}

private struct GeekThreadRows: View {
    @Environment(AppModel.self) private var model
    let controller: GeekController

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
                        ForEach(model.threads) { t in
                            GeekThreadRow(
                                thread: t, cursor: t.id == cursor, marked: marked.contains(t.id), layout: layout,
                                folder: folderLabels.map { $0[t.folderId] ?? .init(folder: "", account: nil) },
                                showsKind: showsKind, suggestion: model.aiSuggestions[t.id])
                                .contentShape(Rectangle())
                                // Click, ⌘-click, ⇧-click (`clickThread`).
                                .onClick { controller.clickThread(t.id, $0) }
                                .uiTarget("thread-\(t.id)")
                                .id(t.id)
                                .task { await model.loadMoreIfNeeded(after: t) }
                        }
                    }
                }
                .scrollIndicators(.never)
                // How many rows fit, for ctrl-d / ctrl-u (half a page). Read
                // again when the window or the text size changes.
                .onChange(of: geo.size.height, initial: true) { _, h in
                    controller.listVisibleRows = GeekController.visibleRows(height: h, rowHeight: scaled(20))
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

/// Fixed column widths of the list rows, computed from the list width in
/// whole characters of the (monospaced) font.
@MainActor
struct RowLayout: Equatable {
    /// Font size of the rows, written for the default text size.
    static let fontSize: CGFloat = 12
    let char: CGFloat
    /// Date: 8 characters ("14/09/21", "16 sept.").
    let date: CGFloat
    /// Sender: about a quarter of the list, between 8 and 24 characters.
    let sender: CGFloat
    /// Message count: 3 characters, right-aligned.
    let count: CGFloat

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
        width - 2 - char - date - sender - icon - folderWidth - count - icon - 8 * char - 6
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

    /// `scale`: the text size factor (`TextSize.scale`).
    init(width: CGFloat, scale: CGFloat? = nil) {
        self.width = width
        char = Fonts.charWidth(Self.fontSize * (scale ?? TextSize.shared.scale))
        date = 8 * char
        let chars = (width / char * 0.26).rounded(.down)
        sender = min(24, max(8, chars)) * char
        count = 3 * char
    }
}

/// The sender column of a list row (`ui.md`, "Liste").
enum GeekSender {
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
private struct GeekThreadRow: View {
    let thread: ThreadSummary
    let cursor: Bool
    let marked: Bool
    let layout: RowLayout
    /// Folder of the row, shown in "Everything" only (`nil` elsewhere).
    var folder: AppModel.FolderLabel? = nil
    /// The icon person / robot before the name (Quarantine only,
    /// `AppModel.listShowsSenderKind`).
    var showsKind = false
    /// The target the AI suggests for the conversation (`a` `s`).
    var suggestion: AISuggestion? = nil

    var body: some View {
        HStack(spacing: layout.char) {
            (marked ? Palette.markedBar : Color.clear).frame(width: 2)
            Text(thread.unreadCount > 0 ? "●" : " ").foregroundStyle(Palette.unread).frame(width: layout.char)
            Text(Format.geekListDate(thread.dateLastReceived)).foregroundStyle(Palette.muted)
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
                        if let symbol = GeekSender.kindSymbol(thread) {
                            Image(systemName: symbol)
                                .font(.system(size: scaled(10), weight: .medium))
                                // The person in the accent colour: among
                                // the robots of Quarantine, the mail that
                                // most likely deserves the Inbox stands out.
                                .foregroundStyle(thread.correspondentKind == "person" ? Palette.accentText : Palette.muted)
                        } else {
                            Color.clear
                        }
                    }
                    .frame(width: layout.icon)
                    // Read once, with the name (`spoken`), not as an image.
                    .accessibilityHidden(true)
                }
                Text(GeekSender.text(thread))
                    .fontWeight(thread.unreadCount > 0 ? .bold : .regular)
                    .lineLimit(1)
                Spacer(minLength: 0)
            }
            .frame(width: layout.sender, alignment: .leading)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(GeekSender.spoken(thread, showsKind: showsKind))
            Group {
                if thread.hasAttachments {
                    Image(systemName: "paperclip")
                        .font(.system(size: scaled(11), weight: .medium))
                        .foregroundStyle(Palette.muted)
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
                ForEach(thread.tags, id: \.self) { Text("#\($0)").foregroundStyle(Palette.accentText) }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            // Muted, fixed width. The account follows only when the
            // subject keeps enough room (`RowLayout.showsAccount`); the
            // reader always shows it.
            if let folder {
                let withAccount = folder.account != nil && layout.showsAccount
                Text(withAccount ? folder.text : folder.folder).foregroundStyle(Palette.muted)
                    .frame(width: layout.folder(withAccount: withAccount), alignment: .leading)
            }
            // The AI's suggestion, before the count: the target's name in
            // the accent colour, after an arrow, as the move it proposes.
            if let s = suggestion {
                Text(AI.suggestionTag(Role.name(s.target), narrow: layout.narrowSuggestion))
                    .foregroundStyle(Palette.accentText)
                    .fixedSize()
                    .accessibilityLabel(L("AI suggests %@", Role.name(s.target)))
            }
            Text(thread.messageCount > 1 ? "\(thread.messageCount)" : "").foregroundStyle(Palette.muted)
                .frame(width: layout.count, alignment: .trailing)
            // The flag: an icon in its own colour, apart from the unread
            // dot's, at the end of the row.
            Group {
                if thread.isFlagged {
                    Image(systemName: "flag.fill")
                        .font(.system(size: scaled(11), weight: .semibold))
                        .foregroundStyle(Palette.flag)
                        .accessibilityLabel(L("flagged"))
                } else {
                    Color.clear
                }
            }
            .frame(width: layout.icon)
        }
        .font(.geek(RowLayout.fontSize, weight: .regular))
        .lineLimit(1)
        .padding(.trailing, 6)
        .frame(height: scaled(20))
        .background(cursor ? Palette.cursor : marked ? Palette.marked : Color.clear)
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
    @ViewBuilder let controls: () -> Controls

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: scaled(8)) {
                subjectText
                labels
                Spacer(minLength: scaled(16))
                controls()
            }
            .frame(height: scaled(26))
            .onAppear { Self.report(lines: 1) }
            VStack(alignment: .leading, spacing: 0) {
                subjectText
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .frame(height: scaled(24))
                // On two lines, the labels open the line of the controls:
                // the subject keeps the whole width.
                HStack(spacing: scaled(8)) {
                    labels
                    Spacer(minLength: 0)
                    controls()
                }
                .frame(height: scaled(22))
                .padding(.bottom, scaled(2))
            }
            .onAppear { Self.report(lines: 2) }
        }
        .padding(.horizontal, 12)
    }

    // Selectable like the body (the text reader is an NSTextView): the
    // subject is often what one copies to search for it or quote it.
    // Swift note: `.textSelection(.enabled)` lets the user drag over a
    // SwiftUI `Text` and copy it with ⌘C; ⌘ shortcuts are left to AppKit
    // by `GeekController.handle`. On two lines, a subject longer than the
    // whole reader is cut at the end; the tooltip shows it whole.
    private var subjectText: some View {
        Text(subject).font(.geek(13, weight: .bold)).lineLimit(1)
            .textSelection(.enabled)
            .help(subject)
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
            .font(.geek(12))
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
private struct GeekReaderColumn: View {
    @Environment(AppModel.self) private var model
    let controller: GeekController

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
                Palette.separator.frame(height: 1)
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
                let label = UnsubscribeDisplay.label(offer)
                HStack(spacing: scaled(4)) {
                    Text(label.text)
                    if let symbol = label.symbol { Image(systemName: symbol) }
                }
                .foregroundStyle(offer.source == "body" ? Palette.muted : Palette.accentText)
                .padding(.horizontal, scaled(4))
                .overlay(Rectangle().stroke(Palette.separator, lineWidth: 1))
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
        label
            .background(model.bodyFormat == format ? Palette.cursor : Color.clear)
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
        body, summary { font-family: "NM Mono", ui-monospace, monospace; }
        body { margin: 16px; font-size: \(Int(scaled(14)))px; }
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
        pre { font: \(Int(scaled(14)))px "NM Mono", ui-monospace, monospace; white-space: pre-wrap; margin: 0 0 8px; }
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
    let controller: GeekController

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSTextView.scrollableTextView()
        scroll.hasVerticalScroller = false
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
        let charWidth = Double(Fonts.charWidth(12 * TextSize.shared.scale))
        let width = Double(scroll.contentSize.width) - 24
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
        weak var controller: GeekController?
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
        paragraph.lineSpacing = 3
        let muted = NSColor(Palette.muted)
        let body: [NSAttributedString.Key: Any] = [.font: Fonts.scaled(14), .foregroundColor: NSColor(Palette.text), .paragraphStyle: paragraph]
        let dim: [NSAttributedString.Key: Any] = [.font: Fonts.scaled(14), .foregroundColor: muted, .paragraphStyle: paragraph]
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
    let controller: GeekController

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
        weak var controller: GeekController?
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
struct VirtualFolderLabel: View {
    let name: String

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: "square.stack").imageScale(.small)
            Text(name)
        }
        .foregroundStyle(Palette.accentText)
    }
}

/// The panel of a choice (`GeekController.activeChoice`): its title and
/// one row per answer, each with its key. Drawn in the bottom-right
/// corner, or by the screen itself where the choice belongs.
struct ChoicePanel: View {
    let controller: GeekController
    let choice: GeekController.Choice

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

    var body: some View {
        // Width of the key column: the longest key, in monospace characters.
        let keyWidth = CGFloat(entries.map(\.key.count).max() ?? 1) * Fonts.charWidth(12 * TextSize.shared.scale) + 4
        VStack(alignment: .leading, spacing: 2) {
            Text(title).foregroundStyle(Palette.accentText).padding(.bottom, 2)
            ForEach(Array(entries.enumerated()), id: \.offset) { i, e in
                HStack(spacing: 10) {
                    Text(e.key).foregroundStyle(Palette.accentText).frame(width: keyWidth, alignment: .leading)
                    if e.virtual {
                        VirtualFolderLabel(name: e.label)
                    } else {
                        Text(e.label).foregroundStyle(e.muted ? Palette.muted : Palette.text)
                    }
                    // Pushes the detail to the right end; at least a gap
                    // of two characters after the longest label.
                    Spacer(minLength: scaled(16))
                    if let detail = e.detail {
                        Text(detail).foregroundStyle(e.detailMuted ? Palette.muted : Palette.unread)
                    }
                }
                // Every row takes the menu's width (that of its widest
                // row), so the details line up on the right edge.
                .frame(maxWidth: .infinity)
                .background(i == highlighted ? Palette.cursor : Color.clear)
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
        .padding(10)
        .background(Palette.popup)
        .overlay(Rectangle().stroke(Palette.separator, lineWidth: 1))
    }
}

/// The list of keys (`?`, `ui.md`): the groups of `Help.groups` side by
/// side in up to three columns, centered, so that every key is seen at
/// once, without scrolling. The window's width decides the number of
/// columns (`Help.columnCount`); `Help.columns` balances them.
private struct HelpPanel: View {
    let groups: [HelpGroup]

    /// Inner margin of the panel, and margin around it in the window.
    private var padding: CGFloat { scaled(14) }
    private var margin: CGFloat { scaled(12) }

    var body: some View {
        let charWidth = Fonts.charWidth(12 * TextSize.shared.scale)
        GeometryReader { geometry in
            let count = Help.columnCount(groups, available: geometry.size.width - 2 * margin,
                                         charWidth: charWidth, padding: padding)
            let columns = Help.columns(groups, count: count)
            VStack(alignment: .leading, spacing: scaled(10)) {
                Text(L("Keys")).foregroundStyle(Palette.accentText)
                // Swift note: `ViewThatFits` shows the first of its views
                // that fits in the space offered. The columns alone almost
                // always fit; in a very short window they scroll instead of
                // being cut.
                ViewThatFits(in: .vertical) {
                    columnsView(columns, charWidth: charWidth)
                    ScrollView { columnsView(columns, charWidth: charWidth) }
                }
            }
            .fixedSize(horizontal: true, vertical: false)
            .padding(padding)
            .background(Palette.popup)
            .overlay(Rectangle().stroke(Palette.separator, lineWidth: 1))
            .padding(margin)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .onAppear { if UIScript.isActive { UIScript.helpColumns = columns.map { $0.map(\.title) } } }
            .onChange(of: count) { _, _ in
                if UIScript.isActive { UIScript.helpColumns = columns.map { $0.map(\.title) } }
            }
        }
    }

    private func columnsView(_ columns: [[HelpGroup]], charWidth: CGFloat) -> some View {
        HStack(alignment: .top, spacing: CGFloat(Help.columnGap) * charWidth) {
            ForEach(Array(columns.enumerated()), id: \.offset) { _, column in
                // One key width for the whole column: the labels of its
                // groups line up, as in a single table.
                let keyWidth = CGFloat(Help.keyWidth(column)) * charWidth + 4
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
    let controller: GeekController
    let kind: GeekController.PickerKind
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
    static func width(_ kind: GeekController.PickerKind, available: CGFloat, scale: CGFloat) -> CGFloat {
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
    static func height(_ kind: GeekController.PickerKind, scale: CGFloat) -> CGFloat {
        switch kind {
        case .attributes:
            (headerHeight + 1 + CGFloat(GeekController.attributeRowCount) * rowHeight
                + CGFloat(GeekController.attributeGapCount) * gapHeight + 4) * scale
        case .attributeValue, .search, .links: 440 * scale
        default: 320 * scale
        }
    }

    /// Height of the input line and of a row, at the default text size.
    static let headerHeight: CGFloat = 24
    static let rowHeight: CGFloat = 20
    /// The line between two groups of rows (`Entry.gapBefore`: the pairs
    /// of the attribute selector), with its space above and below.
    static let gapHeight: CGFloat = 7

    /// `height`, kept inside a window `available` points tall, with a
    /// margin above and below: in a short window, the rows scroll.
    static func height(_ kind: GeekController.PickerKind, scale: CGFloat, available: CGFloat) -> CGFloat {
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
            .padding(.horizontal, 8)
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
                            HStack(spacing: 8) {
                                Text(e.key).foregroundStyle(Palette.muted).frame(width: scaled(22), alignment: .leading)
                                if let icon = e.icon {
                                    Image(systemName: icon).foregroundStyle(Palette.muted).frame(width: scaled(16))
                                }
                                if halves {
                                    // Links: what the mail calls the link on the
                                    // left, where it goes on the right, each in
                                    // half of the row, so that a long text never
                                    // pushes the address out of sight. Two frames
                                    // equally flexible get the same width.
                                    Text(e.label).lineLimit(1).truncationMode(.middle)
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                    Text(e.detail ?? "").lineLimit(1).truncationMode(.middle)
                                        .foregroundStyle(Palette.muted)
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                } else {
                                    Text(e.label).lineLimit(1).truncationMode(.middle)
                                    Spacer()
                                    // At the right end of the row: an unread count
                                    // stands out, an information stays muted.
                                    if let detail = e.detail {
                                        Text(detail).lineLimit(1)
                                            .foregroundStyle(e.detailStyle == .count ? Palette.unread : Palette.muted)
                                    }
                                }
                            }
                            .padding(.horizontal, 8)
                            .frame(height: scaled(Self.rowHeight))
                            .background(i == current ? Palette.cursor : Color.clear)
                            .contentShape(Rectangle())
                            .onTapGesture { controller.clickPicker(kind, i) }
                            .uiTarget("picker-\(i)")
                            .id(i)
                        }
                    }
                }
                .scrollIndicators(.never)
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
        .background(Palette.popup)
        .overlay(Rectangle().stroke(Palette.separator, lineWidth: 1))
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
        }
    }

    private var emptyText: String {
        switch kind {
        case .links: L("No link or attachment in this mail")
        case .tag, .group: L("Type a name")
        case .accounts: L("No account")
        case .attributes: L("No attribute")
        case .ai: L("No AI prompt matches")
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
    @ViewBuilder let row: (Int) -> Row

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(title).font(.geek(13, weight: .bold))
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
                                .frame(height: scaled(20))
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .background(i == index ? Palette.cursor : Color.clear)
                                .contentShape(Rectangle())
                                .onTapGesture { onClick(i) }
                                .uiTarget("screen-\(i)")
                                .id(i)
                        }
                    }
                }
                .scrollIndicators(.never)
                .onChange(of: index) { _, i in proxy.scrollTo(i) }
            }
        }
    }
}

private struct ContactsScreen: View {
    let controller: GeekController

    var body: some View {
        ScreenList(
            title: L("Contacts"), hint: L("/ filter · m: decision · M: domain decision · o: human or robot · return: their mails · esc: back"),
            count: controller.contacts.count, index: controller.screenIndex,
            onClick: { controller.clickScreenRow($0) }
        ) { i in
            let c = controller.contacts[i]
            HStack(spacing: 8) {
                Text(c.addr).frame(width: scaled(300), alignment: .leading)
                Text(c.name ?? "").foregroundStyle(Palette.muted).frame(width: scaled(200), alignment: .leading)
                // The address's decision, or the domain's (marked "@"),
                // which applies when the address has none.
                if let own = c.screeningTarget {
                    Text(Role.name(own)).frame(width: scaled(110), alignment: .leading)
                } else {
                    Text(c.domainTarget.map { "@" + Role.name($0) } ?? "").foregroundStyle(Palette.muted)
                        .frame(width: scaled(110), alignment: .leading)
                }
                // Human or robot; a choice of the user is marked "*".
                Text(c.originLabel).foregroundStyle(c.forced ? Palette.text : Palette.muted)
                    .frame(width: scaled(80), alignment: .leading)
                Text("\(c.messageCount)").foregroundStyle(Palette.muted).frame(width: scaled(60), alignment: .trailing)
                Text(c.groups.map { "@" + $0 }.joined(separator: " ")).foregroundStyle(Palette.accentText)
            }
            .lineLimit(1)
        }
    }
}

private struct HistoryScreen: View {
    let controller: GeekController

    var body: some View {
        ScreenList(
            title: L("History"), hint: L("newest first · esc: back"),
            count: controller.history.count, index: controller.screenIndex,
            onClick: { controller.clickScreenRow($0) }
        ) { i in
            let e = controller.history[i]
            HStack(spacing: 8) {
                Text(Format.longDate(e.date)).foregroundStyle(Palette.muted).frame(width: scaled(230), alignment: .leading)
                Text(e.source).foregroundStyle(Palette.accentText).frame(width: scaled(50), alignment: .leading)
                Text(e.action).frame(width: scaled(110), alignment: .leading)
                Text(e.summary).lineLimit(1)
            }
        }
    }
}

/// Tags or groups: a name and a count per row.
private struct NamesScreen: View {
    let controller: GeekController
    let title: String
    let rows: [(String, String)]

    var body: some View {
        ScreenList(
            title: title, hint: L("return: their mails · r: rename · d: delete · esc: back"), count: rows.count, index: controller.screenIndex,
            onClick: { controller.clickScreenRow($0) }
        ) { i in
            HStack(spacing: 8) {
                Text(rows[i].0).frame(width: scaled(260), alignment: .leading)
                Text(rows[i].1).foregroundStyle(Palette.muted)
            }
        }
    }
}
