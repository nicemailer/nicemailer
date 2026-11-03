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
        VStack(spacing: 0) {
            if controller.statusPosition == .top { GeekStatusBar(controller: controller) }
            ZStack(alignment: .bottomTrailing) {
                content
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
    }

    @ViewBuilder private var content: some View {
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

    @ViewBuilder private var overlay: some View {
        switch controller.overlay {
        case .menu(let prefix):
            KeyMenu(title: menuTitle(prefix), entries: controller.menuEntries(prefix).map { ($0.key, $0.label, $0.disabled) })
                .padding(12)
        case .help:
            KeyMenu(title: L("Keys"), entries: controller.helpLines.map { ($0.0, $0.1, false) })
                .padding(12)
        case .picker(let kind):
            PickerPopup(controller: controller, kind: kind)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .none, .search, .contactFilter:
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
            Text(controller.locationLabel)
                .lineLimit(1)
            if case .search = controller.overlay {
                Text("/" + controller.searchQuery + "▏").foregroundStyle(Palette.accent)
            }
            if case .contactFilter = controller.overlay {
                Text("/" + controller.contactsQuery + "▏").foregroundStyle(Palette.accent)
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
        .frame(height: 20)
        .background(Palette.statusBar)
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
        .coordinateSpace(name: "columns")
    }
}

/// Third column: the folders, grouped by account.
private struct GeekFolderColumn: View {
    @Environment(AppModel.self) private var model
    let controller: GeekController

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                let several = Set(model.folders.map(\.accountId)).count > 1
                ForEach(Array(model.folders.enumerated()), id: \.element.id) { index, f in
                    if several, index == 0 || model.folders[index - 1].accountId != f.accountId {
                        Text(f.accountName).foregroundStyle(Palette.muted).padding(.top, index == 0 ? 4 : 10).padding(.horizontal, 8)
                    }
                    HStack(spacing: 6) {
                        Text(f.letter?.lowercased() ?? " ").foregroundStyle(Palette.muted)
                        Text(f.name)
                        Spacer()
                        if f.unreadCount > 0 { Text("\(f.unreadCount)").foregroundStyle(Palette.unread) }
                    }
                    .padding(.horizontal, 8)
                    .frame(height: 20)
                    .background(f.id == model.selectedFolderId && model.listFilter == nil ? Palette.cursor : Color.clear)
                    // The whole row is clickable, not only its text.
                    .contentShape(Rectangle())
                    .onTapGesture { controller.goToFolder(f.id) }
                }
            }
        }
        .scrollIndicators(.never)
    }
}

/// The mail list, or the search results while a search is active.
private struct GeekThreadColumn: View {
    @Environment(AppModel.self) private var model
    let controller: GeekController

    var body: some View {
        if model.searchText.isEmpty {
            GeekThreadRows(controller: controller)
        } else {
            GeekSearchRows(controller: controller)
        }
    }
}

private struct GeekThreadRows: View {
    @Environment(AppModel.self) private var model
    let controller: GeekController

    var body: some View {
        let cursor = model.selectedThreadId
        let marked = model.marked
        // The column widths depend on the list width only, never on a row's
        // content: every row gets the same layout and the columns line up.
        GeometryReader { geo in
            let layout = RowLayout(width: geo.size.width)
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(model.threads) { t in
                            GeekThreadRow(thread: t, cursor: t.id == cursor, marked: marked.contains(t.id), layout: layout)
                                .contentShape(Rectangle())
                                // Click, ⌘-click, ⇧-click (`clickThread`).
                                .onTapGesture { controller.clickThread(t.id) }
                                .id(t.id)
                                .task { await model.loadMoreIfNeeded(after: t) }
                        }
                    }
                }
                .scrollIndicators(.never)
                // Keep the cursor visible, without animation: `scrollTo`
                // with no anchor scrolls only as much as needed.
                .onChange(of: model.selectedThreadId) { _, id in
                    if let id { proxy.scrollTo(id) }
                }
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
private struct RowLayout: Equatable {
    static let fontSize: CGFloat = 12
    let char: CGFloat
    /// Date: 8 characters ("14/09/21", "16 sept.").
    let date: CGFloat
    /// Sender: about a quarter of the list, between 8 and 24 characters.
    let sender: CGFloat
    /// Message count: 3 characters, right-aligned.
    let count: CGFloat

    init(width: CGFloat) {
        char = Fonts.charWidth(Self.fontSize)
        date = 8 * char
        let chars = (width / char * 0.26).rounded(.down)
        sender = min(24, max(8, chars)) * char
        count = 3 * char
    }
}

/// One list row: mark bar, unread dot, date, sender, subject and tags,
/// count, flag. Every column but the subject has a fixed width.
private struct GeekThreadRow: View {
    let thread: ThreadSummary
    let cursor: Bool
    let marked: Bool
    let layout: RowLayout

    var body: some View {
        HStack(spacing: layout.char) {
            (marked ? Palette.markedBar : Color.clear).frame(width: 2)
            Text(thread.unreadCount > 0 ? "●" : " ").foregroundStyle(Palette.unread).frame(width: layout.char)
            Text(Format.geekListDate(thread.dateLastReceived)).foregroundStyle(Palette.muted)
                .frame(width: layout.date, alignment: .leading)
            Text(thread.participants.last ?? "")
                .fontWeight(thread.unreadCount > 0 ? .bold : .regular)
                .frame(width: layout.sender, alignment: .leading)
            // The subject takes the remaining width; tags follow it and are
            // cut first.
            HStack(spacing: layout.char) {
                Text(thread.subject.isEmpty ? L("(no subject)") : thread.subject).layoutPriority(1)
                ForEach(thread.tags, id: \.self) { Text("#\($0)").foregroundStyle(Palette.accent) }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Text(thread.messageCount > 1 ? "\(thread.messageCount)" : "").foregroundStyle(Palette.muted)
                .frame(width: layout.count, alignment: .trailing)
            Text(thread.isFlagged ? "*" : " ").foregroundStyle(Palette.unread).frame(width: layout.char)
        }
        .font(.geek(RowLayout.fontSize, weight: .regular))
        .lineLimit(1)
        .padding(.trailing, 6)
        .frame(height: 20)
        .background(cursor ? Palette.cursor : marked ? Palette.marked : Color.clear)
    }
}

private struct GeekSearchRows: View {
    @Environment(AppModel.self) private var model
    let controller: GeekController

    var body: some View {
        GeekSearchList(hits: model.searchResults, current: controller.searchIndex) { controller.clickSearch($0) }
    }
}

/// The search results as plain values: when the results or the cursor
/// change, the parameters differ and SwiftUI rebuilds the rows. (Reading
/// the observable model inside the scroll view's closure was not enough:
/// the rows stayed those of the first query.)
private struct GeekSearchList: View {
    let hits: [SearchHit]
    let current: Int
    /// Called with the row's position when it is clicked.
    let onClick: (Int) -> Void

    var body: some View {
        GeometryReader { geo in
        let layout = RowLayout(width: geo.size.width)
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    // Identified by position, like `.id(i)` used to scroll: a
                    // ForEach identity (message id) different from the `.id`
                    // left the lazy stack showing the rows of an older query.
                    ForEach(Array(hits.enumerated()), id: \.offset) { i, hit in
                        GeekSearchRow(hit: hit, cursor: i == current, layout: layout)
                            .contentShape(Rectangle())
                            .onTapGesture { onClick(i) }
                            .id(i)
                    }
                }
            }
            .scrollIndicators(.never)
            .onChange(of: current) { _, i in proxy.scrollTo(i) }
            .overlay {
                if hits.isEmpty { Text(L("No result")).foregroundStyle(Palette.muted) }
            }
        }
        }
    }
}

/// One search result: date, sender, subject, folder, and the matched
/// excerpt. A separate view with plain parameters, so SwiftUI redraws it
/// when the cursor moves (inline row content in a lazy stack was not
/// always refreshed).
private struct GeekSearchRow: View {
    let hit: SearchHit
    let cursor: Bool
    let layout: RowLayout

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: layout.char) {
                Text(Format.geekListDate(hit.dateReceived)).foregroundStyle(Palette.muted)
                    .frame(width: layout.date, alignment: .leading)
                Text(hit.fromName ?? hit.fromAddr ?? "").frame(width: layout.sender, alignment: .leading)
                Text(hit.subject).frame(maxWidth: .infinity, alignment: .leading)
                Text(Role.name(hit.folderRole)).foregroundStyle(Palette.muted)
                    .frame(width: 10 * layout.char, alignment: .trailing)
            }
            // The excerpt starts under the sender column.
            Text(hit.highlight).foregroundStyle(Palette.muted)
                .padding(.leading, layout.date + layout.char)
        }
        .font(.geek(RowLayout.fontSize))
        .lineLimit(1)
        .padding(.horizontal, 6)
        .frame(height: 36)
        .background(cursor ? Palette.cursor : Color.clear)
    }
}

// MARK: - Reader

/// The open thread, as text (iA Writer Mono V) or as sanitized HTML,
/// following the choice remembered for the sender.
private struct GeekReaderColumn: View {
    @Environment(AppModel.self) private var model
    let controller: GeekController

    var body: some View {
        if let thread = model.openThread, thread.id == model.selectedThreadId {
            VStack(spacing: 0) {
                HStack {
                    Text(thread.subject).font(.geek(13, weight: .bold)).lineLimit(1)
                    Spacer()
                    if model.latestReceived?.bodyHtml != nil {
                        // The text / HTML switch (`ui.md`); also `vh`.
                        HStack(spacing: 0) {
                            Text(" text ").background(model.bodyFormat == "text" ? Palette.cursor : Color.clear)
                            Text(" html ").background(model.bodyFormat == "html" ? Palette.cursor : Color.clear)
                        }
                        .foregroundStyle(Palette.muted)
                        .contentShape(Rectangle())
                        .onTapGesture { model.toggleBodyFormat() }
                    }
                }
                .padding(.horizontal, 12)
                .frame(height: 26)
                Palette.separator.frame(height: 1)
                if model.bodyFormat == "html" {
                    HTMLReader(html: threadHTML(thread))
                } else {
                    TextReader(thread: thread, controller: controller)
                }
            }
        } else {
            Text(L("No mail")).foregroundStyle(Palette.muted).frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    /// The thread as one HTML document: headers in monospace, then each
    /// message's sanitized HTML, or its text when it has none.
    private func threadHTML(_ thread: ThreadView) -> String {
        func esc(_ s: String) -> String {
            s.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;")
                .replacingOccurrences(of: ">", with: "&gt;")
        }
        var html = """
        <html><head><meta charset="utf-8"><style>
        \(Fonts.cssFontFaces)
        :root { color-scheme: light dark; }
        /* The mail's own styles are removed by the sanitizer: everything is
           shown in the theme font, like the text reader. */
        body, table, td, p, div, span, a, li { font-family: "NM Mono", ui-monospace, monospace; }
        body { margin: 16px; font-size: 14px; }
        .h { font-size: 11px; color: #888; margin: 0 0 12px; white-space: pre-wrap; }
        pre { font: 14px "NM Mono", ui-monospace, monospace; white-space: pre-wrap; }
        hr { border: 0; border-top: 1px solid #8884; margin: 24px 0; }
        img { max-width: 100%; height: auto; }
        /* Remote images were removed by the core: hide the empty boxes. */
        img:not([src]), img[src^="cid:"] { display: none; }
        /* No scroll bar in the geek theme (ui.md). */
        html { scrollbar-width: none; }
        ::-webkit-scrollbar { display: none; }
        </style></head><body>
        """
        for (i, m) in thread.messages.enumerated() {
            if i > 0 { html += "<hr>" }
            html += "<div class=\"h\">\(esc(header(m)))</div>"
            if let body = m.bodyHtml { html += body } else { html += "<pre>\(esc(m.bodyText ?? ""))</pre>" }
        }
        return html + "</body></html>"
    }
}

/// Header lines of a message, shared by the text and HTML readers.
private func header(_ m: MessageView) -> String {
    let from = m.fromName.map { "\($0) <\(m.fromAddr ?? "")>" } ?? m.fromAddr ?? ""
    var lines = [L("From: %@", from)]
    if !m.to.isEmpty { lines.append(L("To: %@", m.to.map(\.addr).joined(separator: ", "))) }
    if !m.cc.isEmpty { lines.append(L("Cc: %@", m.cc.map(\.addr).joined(separator: ", "))) }
    lines.append(L("Date: %@", Format.longDate(m.dateReceived)) + "   [\(Role.name(m.folderRole))]")
    return lines.joined(separator: "\n")
}

/// Text reader: an AppKit text view, fast with long mails and selectable,
/// without scroll bars. The controller keeps its scroll view to scroll it
/// from the keyboard (space, ctrl-d, ctrl-u).
///
/// Swift note: `NSViewRepresentable` wraps an AppKit view for SwiftUI;
/// `makeNSView` builds it once, `updateNSView` refreshes it when the data
/// changes.
private struct TextReader: NSViewRepresentable {
    let thread: ThreadView
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
            text.isAutomaticLinkDetectionEnabled = true
        }
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        controller.readerScroll = scroll
        guard let text = scroll.documentView as? NSTextView else { return }
        // Rebuild only when the thread changes, so a redraw does not reset
        // the scroll position.
        let key = "\(thread.id)-\(thread.messages.count)-\(thread.messages.last?.id ?? 0)"
        if context.coordinator.key == key { return }
        context.coordinator.key = key
        text.textStorage?.setAttributedString(attributed(thread))
        text.scrollToBeginningOfDocument(nil)
    }

    func makeCoordinator() -> Coordinator { Coordinator() }
    final class Coordinator { var key = "" }

    private func attributed(_ thread: ThreadView) -> NSAttributedString {
        let out = NSMutableAttributedString()
        let headerAttrs: [NSAttributedString.Key: Any] = [
            .font: Fonts.reading(11),
            .foregroundColor: NSColor(Palette.muted),
        ]
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineSpacing = 3
        let bodyAttrs: [NSAttributedString.Key: Any] = [
            .font: Fonts.reading(14),
            .foregroundColor: NSColor(Palette.text),
            .paragraphStyle: paragraph,
        ]
        for (i, m) in thread.messages.enumerated() {
            if i > 0 { out.append(NSAttributedString(string: "\n\n" + String(repeating: "─", count: 40) + "\n\n", attributes: headerAttrs)) }
            out.append(NSAttributedString(string: header(m) + "\n\n", attributes: headerAttrs))
            let body = (m.bodyText ?? L("(body not downloaded)")).trimmingCharacters(in: .whitespacesAndNewlines)
            out.append(NSAttributedString(string: body, attributes: bodyAttrs))
        }
        return out
    }
}

/// HTML reader: a web view without JavaScript; links open in the browser.
private struct HTMLReader: NSViewRepresentable {
    let html: String

    func makeNSView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        // The HTML is already sanitized by the core (no script); turning
        // JavaScript off is a second barrier.
        config.defaultWebpagePreferences.allowsContentJavaScript = false
        let view = WKWebView(frame: .zero, configuration: config)
        view.navigationDelegate = context.coordinator
        view.setValue(false, forKey: "drawsBackground")
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {
        guard context.coordinator.html != html else { return }
        context.coordinator.html = html
        view.loadHTMLString(html, baseURL: nil)
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator: NSObject, WKNavigationDelegate {
        var html = ""
        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                     decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
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
    let entries: [(String, String, Bool)]

    var body: some View {
        // Width of the key column: the longest key, in monospace characters.
        let keyWidth = CGFloat(entries.map(\.0.count).max() ?? 1) * 7.5 + 4
        VStack(alignment: .leading, spacing: 2) {
            Text(title).foregroundStyle(Palette.accent).padding(.bottom, 2)
            ForEach(Array(entries.enumerated()), id: \.offset) { _, e in
                HStack(spacing: 10) {
                    Text(e.0).foregroundStyle(Palette.accent).frame(width: keyWidth, alignment: .leading)
                    Text(e.1).foregroundStyle(e.2 ? Palette.muted : Palette.text)
                }
            }
        }
        .padding(10)
        .background(Palette.popup)
        .overlay(Rectangle().stroke(Palette.separator, lineWidth: 1))
    }
}

/// Helix-style picker: the typed text on top, the filtered rows below.
private struct PickerPopup: View {
    let controller: GeekController
    let kind: GeekController.PickerKind

    var body: some View {
        let entries = controller.pickerEntries(kind)
        let current = controller.pickerIndex
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Text(title).foregroundStyle(Palette.accent)
                Text(controller.pickerQuery + "▏")
                Spacer()
                Text("\(entries.count)").foregroundStyle(Palette.muted)
            }
            .padding(.horizontal, 8)
            .frame(height: 24)
            Palette.separator.frame(height: 1)
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(entries.enumerated()), id: \.offset) { i, e in
                            HStack(spacing: 8) {
                                Text(e.key).foregroundStyle(Palette.muted).frame(width: 22, alignment: .leading)
                                Text(e.label).lineLimit(1).truncationMode(.middle)
                                Spacer()
                            }
                            .padding(.horizontal, 8)
                            .frame(height: 20)
                            .background(i == current ? Palette.cursor : Color.clear)
                            .contentShape(Rectangle())
                            .onTapGesture { controller.clickPicker(kind, i) }
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
        .frame(width: 520, height: 320)
        .background(Palette.popup)
        .overlay(Rectangle().stroke(Palette.separator, lineWidth: 1))
    }

    private var title: String {
        switch kind {
        case .folders: L("Folder")
        case .links: L("Links")
        case .tag: L("Tag")
        case .group: L("Group")
        }
    }

    private var emptyText: String {
        switch kind {
        case .links: L("No link in this mail")
        case .tag, .group: L("Type a name")
        case .folders: L("No folder")
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
            .frame(height: 26)
            Palette.separator.frame(height: 1)
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(0..<count, id: \.self) { i in
                            row(i)
                                .padding(.horizontal, 10)
                                .frame(height: 20)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .background(i == index ? Palette.cursor : Color.clear)
                                .contentShape(Rectangle())
                                .onTapGesture { onClick(i) }
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
            title: L("Contacts"), hint: L("/ filter · return: their mails · esc: back"),
            count: controller.contacts.count, index: controller.screenIndex,
            onClick: { controller.clickScreenRow($0) }
        ) { i in
            let c = controller.contacts[i]
            HStack(spacing: 8) {
                Text(c.addr).frame(width: 300, alignment: .leading)
                Text(c.name ?? "").foregroundStyle(Palette.muted).frame(width: 200, alignment: .leading)
                Text(c.screeningTarget.map { Role.name($0) } ?? "").frame(width: 90, alignment: .leading)
                Text("\(c.messageCount)").foregroundStyle(Palette.muted).frame(width: 60, alignment: .trailing)
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
                Text(Format.longDate(e.date)).foregroundStyle(Palette.muted).frame(width: 230, alignment: .leading)
                Text(e.source).foregroundStyle(Palette.accent).frame(width: 50, alignment: .leading)
                Text(e.action).frame(width: 110, alignment: .leading)
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
                Text(rows[i].0).frame(width: 260, alignment: .leading)
                Text(rows[i].1).foregroundStyle(Palette.muted)
            }
        }
    }
}
