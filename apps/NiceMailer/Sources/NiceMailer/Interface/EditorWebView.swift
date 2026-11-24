import AppKit
import OSLog
import WebKit

// The editor of a mail's text (`specifications/edition.md`): Tiptap, in a
// web view, driven through the protocol of `apps/editor/PROTOCOL.md`.
//
// Writing a mail follows the Mac: the text is edited by a standard text
// component (here WebKit's, with Tiptap's model over it), with the Mac's
// keys, spelling, dictation and Writing Tools. The app injects only its few
// commands (`InterfaceController.handleComposeKey`): every other key goes to the
// web view. The recipients and the subject stay the app's own fields.
//
// One web view lives for the whole life of the app: created and loaded once
// the first list is shown (`AppModel.ensureEditor`), hidden, so that `r`
// opens the editor without waiting for WebKit.
// Each opening of a reply sends `load`.
//
// Swift note: `@MainActor` makes every method run on the main thread,
// where AppKit and WebKit must be used; `WKScriptMessageHandler` is the
// protocol WebKit calls when the page posts a message.

/// What the editor posted last about its cursor (`EditorState` of the
/// protocol).
struct EditorStateView: Decodable, Equatable {
    struct Marks: Decodable, Equatable {
        var bold = false
        var italic = false
        var underline = false
        var link = false
    }
    struct Range: Decodable, Equatable {
        var start: Int
        var end: Int
    }
    var modal = false
    var focused = false
    var textRange: Range?
    var empty = true
    var selection = ""
    var marks = Marks()
    var block = "paragraph"
    var bulletList = false
    var orderedList = false
    var blockquote = false

    init() {}
}

/// The content of the editor at a moment: the text (text only) or the
/// HTML (formatted), its mode, and the editor's revision.
struct EditorContent: Equatable {
    let content: String
    let mode: ComposeFormat
    let revision: Int
}

/// The mode of the text of a draft (`draft.body_format`).
enum ComposeFormat: String, Equatable {
    case text
    case html
}

/// The lists of the formatted mode, as the core gives them
/// (`compose_formats`, `nm_core::formats`).
struct ComposeFormats: Decodable, Equatable {
    struct Font: Decodable, Equatable { let name: String; let stack: String }
    struct Size: Decodable, Equatable { let key: String; let css: String }
    struct Color: Decodable, Equatable { let key: String; let hex: String }
    let fonts: [Font]
    let sizes: [Size]
    let colors: [Color]
    let defaultFormat: String

    static let empty = ComposeFormats(fonts: [], sizes: [], colors: [], defaultFormat: "text")
}

/// What the reply needs of the editor of its text. The app's is
/// `EditorBridge`; the tests give a fake one.
@MainActor
protocol MailTextEditor: AnyObject {
    /// The last state the editor posted.
    var state: EditorStateView { get }
    /// Opens a draft: its content in `mode`; `fromText`: a text to show
    /// formatted (a proposal, in the formatted mode of `compose.format`).
    /// `cursor`: in text only, where the cursor goes (UTF-16 units).
    func load(mode: ComposeFormat, content: String, fromText: Bool, cursor: Int, focus: Bool, formats: ComposeFormats)
    /// Switches the mode; `content` is the text of the draft in both cases
    /// (`edition.md`, section 3).
    func setMode(_ mode: ComposeFormat, content: String)
    /// Puts a text in place of the selection, in one step of undo.
    func replaceSelection(text: String)
    /// Replaces a range of the document's text (UTF-16 units: the text in
    /// text only, a line per text block formatted), in one step of undo;
    /// formatted, the new text keeps the formatting of the passage.
    func replaceRange(start: Int, end: Int, text: String)
    /// Formatted mode: the whole HTML replaced, in one step of undo (the
    /// user's blocks the AI rewrote).
    func replaceContent(html: String)
    /// Gives the keys to the text, or takes them back.
    func focus()
    func blur()
    /// The content as it is now, the last key typed included; `nil` if the
    /// editor did not answer in time.
    func flush() async -> EditorContent?
    /// A key for the text while the editor cannot take it yet (its page is
    /// still loading, just after the launch): kept and typed once it can.
    /// True when kept; false when the editor takes it itself.
    func keepKeyUntilReady(_ event: NSEvent) -> Bool
}

/// The app's editor: the web view, its page, and the messages.
@MainActor
final class EditorBridge: NSObject, MailTextEditor, WKScriptMessageHandler, WKNavigationDelegate {
    /// The editor of the app, created once (`AppModel.ensureEditor`).
    static let shared = EditorBridge()

    let webView: EditorWebView
    private(set) var state = EditorStateView()
    /// The page said it is ready, with our version of the protocol.
    private(set) var isReady = false
    /// Messages sent before the page was ready, in order.
    private var pending: [[String: Any]] = []
    private var flushes: [Int: CheckedContinuation<EditorContent?, Never>] = [:]
    private var nextFlush = 1

    /// The draft's content changed (after the editor's pause).
    var onChanged: ((EditorContent) -> Void)?
    /// What is under the cursor changed.
    var onState: ((EditorStateView) -> Void)?
    /// `⇧⇥` outside a list: the keys go back to the subject.
    var onLeave: (() -> Void)?
    /// Files dropped on the text: attached, never put in the text.
    var onFiles: (([URL]) -> Void)?

    /// Version of the protocol (`apps/editor/src/protocol.ts`).
    static let protocolVersion = 1
    /// How long `flush` waits for the page (`edition.md`, section 8).
    static let flushTimeout: Duration = .seconds(1)
    /// The scheme the page is served under: an origin of its own, so that
    /// its Content Security Policy (`script-src 'self'`) means the bundle.
    static let scheme = "nm-editor"

    private static let log = Logger(subsystem: "NiceMailer", category: "Editor")

    override init() {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.setURLSchemeHandler(EditorFiles(), forURLScheme: Self.scheme)
        // The app's bundled font, which the web process does not see, as
        // data URLs (`Fonts.cssFontFaces`), before the page's own styles.
        let fonts = "const s=document.createElement('style');s.textContent=\(Self.jsString(Fonts.cssFontFaces));document.documentElement.appendChild(s);"
        config.userContentController.addUserScript(WKUserScript(source: fonts, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        webView = EditorWebView(frame: .zero, configuration: config)
        super.init()
        // Swift note: the controller holds its handler strongly; a weak
        // proxy keeps the bridge from holding itself through WebKit.
        config.userContentController.add(WeakHandler(self), name: "editor")
        webView.navigationDelegate = self
        webView.setValue(false, forKey: "drawsBackground")
        webView.onFiles = { [weak self] urls in self?.onFiles?(urls) }
        webView.allowsMagnification = false
        if UIScript.isActive { Self.treatAsVisible(webView) }
        // The fonts of the bar's menu, read in the background meanwhile.
        MacFonts.prepare()
        Task { @MainActor in
            // The same block list as the reader: no remote load at all.
            if let list = await RemoteBlocker.list() { config.userContentController.add(list) }
            if let url = URL(string: "\(Self.scheme)://editor/index.html") { self.webView.load(URLRequest(url: url)) }
        }
    }

    /// UI script mode only: the window is off screen and transparent, and
    /// WebKit, seeing it occluded, slows the page's timers down (the
    /// editor's pause of typing could then last seconds) and draws no frame.
    /// WebKit's private switch for the occlusion detection, called only if
    /// it exists; never in the app as the user runs it.
    private static func treatAsVisible(_ web: WKWebView) {
        let selector = NSSelectorFromString("_setWindowOcclusionDetectionEnabled:")
        guard web.responds(to: selector), let method = class_getInstanceMethod(type(of: web), selector) else { return }
        // Swift note: the method takes a C `BOOL`, which `perform(_:with:)`
        // cannot pass; its implementation is called as a C function.
        typealias Setter = @convention(c) (AnyObject, Selector, Bool) -> Void
        unsafeBitCast(method_getImplementation(method), to: Setter.self)(web, selector, false)
    }

    /// Text typed before the page was ready, typed into it when it is.
    private var early = ""

    // MARK: MailTextEditor

    func keepKeyUntilReady(_ event: NSEvent) -> Bool {
        guard !isReady else { return false }
        let mods = event.modifierFlags.intersection([.command, .control, .option])
        guard mods.isEmpty || mods == .option else { return true }
        switch event.keyCode {
        case KeyCode.returnKey: early += "\n"
        case KeyCode.delete: if !early.isEmpty { early.removeLast() }
        default:
            if let c = event.characters, LineEdit.isTypable(c) { early += c }
        }
        return true
    }

    func load(mode: ComposeFormat, content: String, fromText: Bool, cursor: Int, focus: Bool, formats: ComposeFormats) {
        // A new draft: the answers to the flushes of the last one are moot.
        for (_, c) in flushes { c.resume(returning: nil) }
        flushes = [:]
        state = EditorStateView()
        sentTheme = nil
        send([
            "type": "load", "v": Self.protocolVersion, "mode": mode.rawValue, "content": content,
            "theme": Self.theme(webView.window?.effectiveAppearance ?? webView.effectiveAppearance), "strings": Self.strings,
            "formats": Self.json(formats, fonts: MacFonts.fonts(known: formats.fonts)),
            "recentFonts": MacFonts.recent, "focus": focus,
            "fromText": fromText, "cursor": cursor,
        ])
        if focus { takeFocus() }
    }

    func setMode(_ mode: ComposeFormat, content: String) {
        send(["type": "setMode", "mode": mode.rawValue, "content": content])
    }

    func replaceSelection(text: String) {
        send(["type": "replaceSelection", "text": text])
    }

    func replaceRange(start: Int, end: Int, text: String) {
        send(["type": "replaceRange", "start": start, "end": end, "text": text])
    }

    func replaceContent(html: String) {
        send(["type": "replaceContent", "html": html])
    }

    func focus() {
        takeFocus()
        send(["type": "focus"])
    }

    func blur() {
        send(["type": "blur"])
        if let w = webView.window, w.firstResponder === webView { w.makeFirstResponder(nil) }
    }

    func flush() async -> EditorContent? {
        // A page not ready yet holds nothing typed: revision 0 says that
        // the model's text is the draft's.
        guard isReady else { return EditorContent(content: "", mode: .text, revision: 0) }
        let id = nextFlush
        nextFlush += 1
        // Swift note: a continuation turns the answer, which comes later
        // as a message, into the result of this `async` function.
        return await withCheckedContinuation { (c: CheckedContinuation<EditorContent?, Never>) in
            flushes[id] = c
            send(["type": "flush", "id": id])
            Task { @MainActor in
                try? await Task.sleep(for: Self.flushTimeout)
                self.flushes.removeValue(forKey: id)?.resume(returning: nil)
            }
        }
    }

    /// The theme last sent: light or dark, and the text size.
    private var sentTheme: String?

    /// Sends the theme if it changed (light or dark, the text size), as the
    /// view in its window shows it: before it is in a window, a web view
    /// takes the Mac's appearance, which may not be the app's.
    func themeChanged() {
        let appearance = webView.window?.effectiveAppearance ?? webView.effectiveAppearance
        let dark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        let key = "\(dark) \(TextSize.shared.scale)"
        guard key != sentTheme else { return }
        sentTheme = key
        send(["type": "theme", "theme": Self.theme(appearance)])
    }

    // MARK: Messages

    private func takeFocus() {
        if let w = webView.window, w.firstResponder !== webView { w.makeFirstResponder(webView) }
    }

    private func send(_ message: [String: Any]) {
        guard isReady else {
            // `load` replaces whatever was waiting: only the last draft matters.
            if message["type"] as? String == "load" { pending.removeAll() }
            pending.append(message)
            return
        }
        guard let data = try? JSONSerialization.data(withJSONObject: message),
              let json = String(data: data, encoding: .utf8) else { return }
        webView.evaluateJavaScript("window.nicemailerEditor && window.nicemailerEditor.receive(\(json))") { _, error in
            if let error { Self.log.error("editor: \(error.localizedDescription, privacy: .public)") }
        }
    }

    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        receive(message.body)
    }

    /// Messages received by type, for the UI scenarios.
    private(set) var received: [String: Int] = [:]

    private func receive(_ body: Any) {
        guard let dict = body as? [String: Any], let type = dict["type"] as? String else { return }
        received[type, default: 0] += 1
        switch type {
        case "ready":
            guard dict["v"] as? Int == Self.protocolVersion else {
                Self.log.error("editor of protocol \(String(describing: dict["v"]), privacy: .public), \(Self.protocolVersion) expected")
                return
            }
            isReady = true
            let waiting = pending
            pending = []
            for m in waiting { send(m) }
            // What was typed meanwhile, at the cursor, as if typed now.
            if !early.isEmpty {
                send(["type": "replaceSelection", "text": early])
                early = ""
            }
        case "changed":
            if let c = Self.content(dict) { onChanged?(c) }
        case "flushed":
            if let id = dict["id"] as? Int, let c = Self.content(dict) { flushes.removeValue(forKey: id)?.resume(returning: c) }
        case "state":
            if let data = try? JSONSerialization.data(withJSONObject: dict),
               let s = try? JSONDecoder().decode(EditorStateView.self, from: data) {
                state = s
                onState?(s)
            }
        case "leave":
            onLeave?()
        case "fontUsed":
            if let name = dict["name"] as? String { MacFonts.used(name) }
        default:
            break
        }
    }

    private static func content(_ dict: [String: Any]) -> EditorContent? {
        guard let content = dict["content"] as? String, let mode = (dict["mode"] as? String).flatMap(ComposeFormat.init) else { return nil }
        return EditorContent(content: content, mode: mode, revision: dict["revision"] as? Int ?? 0)
    }

    // MARK: Navigation

    /// Only the editor's own page is ever loaded: a link clicked in the text
    /// opens nothing (`edition.md`, section 4).
    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                 decisionHandler: @escaping @MainActor @Sendable (WKNavigationActionPolicy) -> Void) {
        let ok = action.request.url?.scheme == Self.scheme && action.targetFrame?.isMainFrame == true
        decisionHandler(ok ? .allow : .cancel)
    }

    /// WebKit's process for the page stopped (memory, a crash): the page is
    /// loaded again; the draft in the core is at its last save.
    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        isReady = false
        if let url = URL(string: "\(Self.scheme)://editor/index.html") { self.webView.load(URLRequest(url: url)) }
    }

    // MARK: Look and texts

    private static func json(_ formats: ComposeFormats, fonts: [ComposeFormats.Font]) -> [String: Any] {
        [
            "fonts": fonts.map { ["name": $0.name, "stack": $0.stack] },
            "sizes": formats.sizes.map { ["key": $0.key, "css": $0.css] },
            "colors": formats.colors.map { ["key": $0.key, "hex": $0.hex] },
        ]
    }

    /// The colours, font and size of the theme, as CSS values: the
    /// palette's own values for the appearance (a dynamic colour converted
    /// out of its view does not reliably follow an appearance asked for).
    static func theme(_ appearance: NSAppearance) -> [String: Any] {
        let dark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        return [
            "dark": dark, "font": Fonts.css(ThemeStore.active.fonts.reading),
            "size": Double(14 * TextSize.shared.scale * CGFloat(ThemeStore.active.fonts.readingScale)),
            "text": Palette.hex(\.text, dark: dark), "background": Palette.hex(\.background, dark: dark),
            "muted": Palette.hex(\.muted, dark: dark), "accent": Palette.hex(\.accentText, dark: dark),
            "selection": Palette.hex(\.selection, dark: dark), "border": Palette.hex(\.separator, dark: dark),
            "panel": Palette.hex(\.bar, dark: dark),
        ]
    }

    /// The texts of the editor's interface, in the app's language (String
    /// Catalog): the editor has no translation of its own.
    static var strings: [String: String] {
        var s: [String: String] = [
            "text.label": L("Text of the mail"),
            "toolbar.font": L("Font"), "font.default": L("Default font"),
            "font.recent": L("Recently used"), "font.all": L("All fonts"),
            "toolbar.size": L("Size"), "size.normal": L("Normal size"),
            "toolbar.color": L("Colour"), "color.default": L("Default colour"),
            "toolbar.bold": L("Bold (⌘B)"), "toolbar.italic": L("Italic (⌘I)"), "toolbar.underline": L("Underline (⌘U)"),
            "toolbar.h1": L("Heading 1 (⌥⌘1)"), "toolbar.h2": L("Heading 2 (⌥⌘2)"), "toolbar.h3": L("Heading 3 (⌥⌘3)"),
            "toolbar.bulletList": L("Bulleted list (⇧⌘8)"), "toolbar.orderedList": L("Numbered list (⇧⌘7)"),
            "toolbar.blockquote": L("Quotation (⇧⌘B)"), "toolbar.link": L("Link (⌘K)"),
            "link.url": L("Address of the link"), "link.text": L("Text of the link"),
            "link.invalid": L("Not a link: http, https or mailto only"),
        ]
        for key in ["gray", "red", "orange", "green", "teal", "blue", "purple", "pink"] {
            s["color.\(key)"] = colorName(key)
        }
        return s
    }

    private static func colorName(_ key: String) -> String {
        switch key {
        case "gray": L("Grey")
        case "red": L("Red")
        case "orange": L("Orange")
        case "green": L("Green")
        case "teal": L("Teal")
        case "blue": L("Blue")
        case "purple": L("Purple")
        case "pink": L("Pink")
        default: key
        }
    }

    /// A Swift string as a JavaScript string literal.
    static func jsString(_ s: String) -> String {
        let data = (try? JSONSerialization.data(withJSONObject: [s])) ?? Data("[\"\"]".utf8)
        let array = String(decoding: data, as: UTF8.self)
        return String(array.dropFirst().dropLast())
    }
}

/// The web view of the editor: files dropped on it are attachments.
final class EditorWebView: WKWebView {
    var onFiles: (([URL]) -> Void)?

    private func files(_ info: NSDraggingInfo) -> [URL] {
        (info.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL]) ?? []
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        files(sender).isEmpty ? super.draggingEntered(sender) : .copy
    }

    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation {
        files(sender).isEmpty ? super.draggingUpdated(sender) : .copy
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        let urls = files(sender)
        guard !urls.isEmpty else { return super.performDragOperation(sender) }
        onFiles?(urls)
        return true
    }
}

/// Serves the editor's page from the app's resources (`Resources/Editor`,
/// built by `make editor`), and nothing else.
private final class EditorFiles: NSObject, WKURLSchemeHandler {
    func webView(_ webView: WKWebView, start task: any WKURLSchemeTask) {
        let name = task.request.url?.lastPathComponent ?? ""
        let types = ["index.html": "text/html", "editor.js": "text/javascript", "editor.css": "text/css"]
        guard let type = types[name],
              let dir = AppResources.bundle.url(forResource: "Editor", withExtension: nil),
              let data = try? Data(contentsOf: dir.appendingPathComponent(name)),
              let url = task.request.url else {
            task.didFailWithError(URLError(.fileDoesNotExist))
            return
        }
        task.didReceive(URLResponse(url: url, mimeType: type, expectedContentLength: data.count, textEncodingName: "utf-8"))
        task.didReceive(data)
        task.didFinish()
    }

    func webView(_ webView: WKWebView, stop task: any WKURLSchemeTask) {}
}

/// Forwards the page's messages to the bridge without WebKit keeping it.
private final class WeakHandler: NSObject, WKScriptMessageHandler {
    weak var target: (any WKScriptMessageHandler)?
    init(_ target: any WKScriptMessageHandler) { self.target = target }
    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        target?.userContentController(controller, didReceive: message)
    }
}
