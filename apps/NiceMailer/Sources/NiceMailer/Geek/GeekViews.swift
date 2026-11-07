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
        if case .failed(let message) = model.status, !UIScript.isActive {
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
                rows: controller.tags.map { ($0.name, L("%lld mails", $0.messageCount)) })
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
                // which no panel covers (pickers are centered, menus drop
                // from the status bar's left part, help sits bottom-right).
                .overlay(alignment: .topTrailing) {
                    Color.clear.frame(width: 16, height: 16).padding(8).uiTarget("outside-panel")
                }
        }
    }

    @ViewBuilder private var overlay: some View {
        // A question (a suspect move to confirm, a decision to choose)
        // comes before any other panel, in the bottom-right corner.
        if let choice = controller.activeChoice {
            KeyMenu(title: controller.choiceTitle(choice),
                    entries: controller.choiceEntries(choice).map { .init(key: $0.key, label: $0.label, detail: $0.detail) },
                    highlighted: controller.choiceIndex,
                    onSelect: { controller.chooseChoiceEntry($0) })
                .padding(12)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
        } else {
            menusAndPickers
        }
    }

    @ViewBuilder private var menusAndPickers: some View {
        switch controller.overlay {
        case .menu(let prefix):
            let menu = KeyMenu(title: menuTitle(prefix),
                               entries: controller.menuEntries(prefix).map {
                                   .init(key: $0.key, label: $0.label, muted: $0.disabled, gapBefore: $0.gapBefore, detail: $0.detail,
                                         detailMuted: $0.detailStyle == .info)
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
            KeyMenu(title: L("Keys"), entries: controller.helpLines.map { .init(key: $0.0, label: $0.1) }, onSelect: nil)
                .padding(12)
        case .picker(let kind):
            // Swift note: a `GeometryReader` gives its content the size it
            // is offered, here the window's content: the links panel takes
            // as much of its width as is useful.
            GeometryReader { geometry in
                PickerPopup(controller: controller, kind: kind,
                            width: PickerPopup.width(kind, available: geometry.size.width, scale: TextSize.shared.scale))
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

/// Compact status bar (`ui.md`). Each element opens with one key: the mode
/// with `?` (the keys), the location with `h` (the folders).
struct GeekStatusBar: View {
    @Environment(AppModel.self) private var model
    let controller: GeekController

    var body: some View {
        HStack(spacing: 10) {
            Text(" \(controller.modeLabel) ")
                .font(.geek(11, weight: .bold))
                .foregroundStyle(Palette.background)
                .background(Palette.accent)
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
                StatusBlock(icon: "magnifyingglass", text: "", key: "F")
                    .onTapGesture { controller.openPickerFromStatusBar(.attributes) }
                    .uiTarget("status-attributes")
                ForEach(model.attributes.chips) { chip in
                    Text(chip.label + " ×")
                        .lineLimit(1)
                        .foregroundStyle(Palette.accent)
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
                ForEach([GeekController.Prefix.view, .move, .save], id: \.self) { p in
                    StatusBlock(icon: p.icon, text: "", key: p.rawValue)
                        .menuAnchor(p, controller)
                        .onTapGesture { controller.openMenuFromStatusBar(p) }
                        .uiTarget("status-\(p.rawValue)")
                        .padding(.leading, p == .save ? 0 : scaled(28))
                }
            }
            if case .contactFilter = controller.overlay {
                HStack(spacing: 0) {
                    Text("/")
                    CaretText(line: controller.contactsLine)
                }
                .foregroundStyle(Palette.accent)
            }
            Spacer(minLength: 8)
            if let error = model.lastError {
                Text(error).foregroundStyle(Palette.error).lineLimit(1)
            } else if let flash = model.flash {
                Text(flash).foregroundStyle(Palette.muted).lineLimit(1).truncationMode(.middle)
            } else if case .failed(let message) = model.status {
                Text(message).foregroundStyle(Palette.error).lineLimit(1)
            }
            if !model.marked.isEmpty {
                Text(L("%lld marked", model.marked.count)).foregroundStyle(Palette.markedBar)
            }
            if model.sort == "modified" { Text(L("by modification")).foregroundStyle(Palette.muted) }
        }
        .font(.geek(11))
        .padding(.horizontal, 6)
        .frame(height: scaled(20))
        .background(Palette.statusBar)
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
    let icon: String
    let text: String
    let key: String

    var body: some View {
        HStack(spacing: scaled(4)) {
            Image(systemName: icon)
                .font(.system(size: scaled(10), weight: .semibold))
                .foregroundStyle(Palette.muted)
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
                        Text(f.name)
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
            }
        }
        .scrollIndicators(.never)
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
                                folder: folderLabels.map { $0[t.folderId] ?? .init(folder: "", account: nil) })
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
                        Text(L("No mail")).foregroundStyle(Palette.muted)
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

    var body: some View {
        HStack(spacing: layout.char) {
            (marked ? Palette.markedBar : Color.clear).frame(width: 2)
            Text(thread.unreadCount > 0 ? "●" : " ").foregroundStyle(Palette.unread).frame(width: layout.char)
            Text(Format.geekListDate(thread.dateLastReceived)).foregroundStyle(Palette.muted)
                .frame(width: layout.date, alignment: .leading)
            Text(thread.participants.last ?? "")
                .fontWeight(thread.unreadCount > 0 ? .bold : .regular)
                .frame(width: layout.sender, alignment: .leading)
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
                ForEach(thread.tags, id: \.self) { Text("#\($0)").foregroundStyle(Palette.accent) }
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
    @ViewBuilder let controls: () -> Controls

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: scaled(8)) {
                subjectText
                Spacer(minLength: scaled(16))
                controls()
            }
            .frame(height: scaled(26))
            .onAppear { Self.report(lines: 1) }
            VStack(alignment: .leading, spacing: 0) {
                subjectText
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .frame(height: scaled(24))
                HStack(spacing: scaled(8)) {
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
            let blocks = ThreadDisplay.blocks(thread, state)
            VStack(spacing: 0) {
                // The subject, then the controls of the mail; on two lines
                // when the subject would not fit whole beside them
                // (`ReaderTitleBar`).
                ReaderTitleBar(subject: thread.subject) { titleControls(state) }
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
            Text("T").foregroundStyle(Palette.accent)
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
                .foregroundStyle(offer.source == "body" ? Palette.muted : Palette.accent)
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
            var summary = unfolded ? "\(arrow)\(who) · \(date)\(address)"
                : "\(arrow)\(who) · \(date)\(domain) — \(esc(ThreadDisplay.snippet(m)))"
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
            case .header(let id, let folded, let title, let detail, let files, let address, let alert, let foldable):
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
    static func symbol(_ name: String, size: CGFloat, color: NSColor, link: String? = nil) -> NSAttributedString {
        let config = NSImage.SymbolConfiguration(pointSize: size, weight: .regular)
            .applying(NSImage.SymbolConfiguration(paletteColors: [color]))
        guard let image = NSImage(systemSymbolName: name, accessibilityDescription: nil)?.withSymbolConfiguration(config)
        else { return NSAttributedString() }
        let attachment = NSTextAttachment()
        attachment.image = image
        attachment.bounds = CGRect(x: 0, y: -size * 0.2, width: image.size.width, height: image.size.height)
        let s = NSMutableAttributedString(attachment: attachment)
        if let link, let url = URL(string: link) {
            s.addAttribute(.link, value: url, range: NSRange(location: 0, length: s.length))
        }
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
        var n = 0, files = 0, senders = 0
        storage.enumerateAttribute(.link, in: NSRange(location: 0, length: storage.length)) { value, range, _ in
            guard let url = value as? URL, url.scheme == "nm" else { return }
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
            // the default browser.
            if action.navigationType == .linkActivated, let url = action.request.url {
                NSWorkspace.shared.open(url)
                decisionHandler(.cancel)
            } else {
                decisionHandler(.allow)
            }
        }
    }
}

// MARK: - Menus and pickers

/// Helix-style key menu, in the bottom-right corner: one line per key.
private struct KeyMenu: View {
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
    }
    /// Called with the row clicked; `nil` for a menu that only informs
    /// (the help).
    let onSelect: ((Int) -> Void)?

    var body: some View {
        // Width of the key column: the longest key, in monospace characters.
        let keyWidth = CGFloat(entries.map(\.key.count).max() ?? 1) * Fonts.charWidth(12 * TextSize.shared.scale) + 4
        VStack(alignment: .leading, spacing: 2) {
            Text(title).foregroundStyle(Palette.accent).padding(.bottom, 2)
            ForEach(Array(entries.enumerated()), id: \.offset) { i, e in
                HStack(spacing: 10) {
                    Text(e.key).foregroundStyle(Palette.accent).frame(width: keyWidth, alignment: .leading)
                    Text(e.label).foregroundStyle(e.muted ? Palette.muted : Palette.text)
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

/// Helix-style picker: the typed text on top, the filtered rows below.
struct PickerPopup: View {
    let controller: GeekController
    let kind: GeekController.PickerKind
    /// Width of the panel (`width(_:available:scale:)`).
    let width: CGFloat

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
        case .attributeValue, .search: wanted = 680 * scale
        default: wanted = 520 * scale
        }
        return min(wanted, available)
    }

    var body: some View {
        let entries = controller.pickerEntries(kind)
        let current = controller.pickerIndex
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Text(title).foregroundStyle(Palette.accent)
                // The caret is drawn where the cursor is (`LineEdit`); a
                // selected text (the current words of Subject…, Text…) is
                // highlighted, since typing will replace it.
                if controller.pickerLine.selectedAll {
                    Text(controller.pickerLine.text).background(Palette.cursor)
                } else {
                    CaretText(line: controller.pickerLine)
                }
                // What can be typed, until something is (`pickerHint`).
                if controller.pickerQuery.isEmpty, let hint = controller.pickerHint(kind) {
                    Text(hint).foregroundStyle(Palette.muted).lineLimit(1).truncationMode(.tail)
                }
                Spacer()
                Text("\(entries.count)").foregroundStyle(Palette.muted)
            }
            .padding(.horizontal, 8)
            .frame(height: scaled(24))
            .uiTarget("picker-header")
            Palette.separator.frame(height: 1)
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(entries.enumerated()), id: \.offset) { i, e in
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
                            .frame(height: scaled(20))
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
        .frame(width: width, height: scaled(expanded ? 440 : 320))
        .onAppear { if UIScript.isActive { UIScript.pickerWidth = width } }
        .onChange(of: width) { _, w in if UIScript.isActive { UIScript.pickerWidth = w } }
        .background(Palette.popup)
        .overlay(Rectangle().stroke(Palette.separator, lineWidth: 1))
    }

    /// Rows in two halves, text and address (the links of `l`).
    private var halves: Bool { kind == .links }

    private var expanded: Bool {
        switch kind {
        case .attributeValue, .search, .links: return true
        default: return false
        }
    }

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
        }
    }

    private var emptyText: String {
        switch kind {
        case .links: L("No link or attachment in this mail")
        case .tag, .group: L("Type a name")
        case .accounts: L("No account")
        case .attributes: L("No attribute")
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
            title: L("Contacts"), hint: L("/ filter · m: decision · M: domain decision · return: their mails · esc: back"),
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
                Text("\(c.messageCount)").foregroundStyle(Palette.muted).frame(width: scaled(60), alignment: .trailing)
                Text(c.groups.map { "@" + $0 }.joined(separator: " ")).foregroundStyle(Palette.accent)
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
                Text(e.source).foregroundStyle(Palette.accent).frame(width: scaled(50), alignment: .leading)
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
            title: title, hint: L("return: their mails · esc: back"), count: rows.count, index: controller.screenIndex,
            onClick: { controller.clickScreenRow($0) }
        ) { i in
            HStack(spacing: 8) {
                Text(rows[i].0).frame(width: scaled(260), alignment: .leading)
                Text(rows[i].1).foregroundStyle(Palette.muted)
            }
        }
    }
}
