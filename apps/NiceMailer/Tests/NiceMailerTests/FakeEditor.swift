import AppKit
@testable import NiceMailer

/// The editor of a reply's text, as the tests see it (`MailTextEditor`).
///
/// The real one is Tiptap in a web view (`EditorBridge`): its editing rules
/// are tested in `apps/editor` (Vitest), and with real keys in the UI
/// scenarios. Here only what the app does with it matters: what it loads,
/// the keys it lets through, the content it gets back, and when.
///
/// A key the app lets through is "typed" at the cursor, and reported at
/// once (`changed`): the editor's own pause is the editor's business.
@MainActor
final class FakeEditor: MailTextEditor {
    struct Load: Equatable {
        let mode: ComposeFormat
        let content: String
        let fromText: Bool
        let cursor: Int
        let focus: Bool
    }

    var state = EditorStateView()
    private(set) var content = ""
    private(set) var mode: ComposeFormat = .text
    /// The cursor, in UTF-16 units of `content`.
    var cursor = 0
    private(set) var revision = 0
    private(set) var loads: [Load] = []
    private(set) var modes: [(mode: ComposeFormat, content: String)] = []
    private(set) var replacements: [String] = []
    private(set) var ranges: [(start: Int, end: Int, text: String)] = []
    private(set) var keys: [String] = []
    private(set) var focusCount = 0
    private(set) var blurCount = 0
    private(set) var flushCount = 0
    /// The editor answers `flush`; false: it does not, in time.
    var answersFlush = true
    /// Reports each change at once (`changed`); false: the last keys are
    /// still in the editor, as during its pause, until a `flush`.
    var reportsChanges = true
    weak var model: AppModel?

    init(model: AppModel) {
        self.model = model
        model.editor = self
    }

    /// `⇧⇥` outside a list (the editor's `leave`).
    var onLeave: (() -> Void)?

    func load(mode: ComposeFormat, content: String, fromText: Bool, cursor: Int, focus: Bool, formats: ComposeFormats) {
        loads.append(Load(mode: mode, content: content, fromText: fromText, cursor: cursor, focus: focus))
        self.mode = mode
        self.content = fromText ? Self.html(content) : content
        self.cursor = cursor
        revision = 0
    }

    func setMode(_ mode: ComposeFormat, content: String) {
        modes.append((mode, content))
        self.mode = mode
        self.content = mode == .html ? Self.html(content) : content
        cursor = 0
        revision += 1
        report()
    }

    func replaceSelection(text: String) {
        replacements.append(text)
        insert(text)
    }

    func replaceRange(start: Int, end: Int, text: String) {
        ranges.append((start, end, text))
        let s = content as NSString
        content = s.replacingCharacters(in: NSRange(location: start, length: end - start), with: text)
        cursor = start + text.utf16.count
        revision += 1
        report()
    }

    func keepKeyUntilReady(_ event: NSEvent) -> Bool { false }

    func focus() { focusCount += 1 }
    func blur() { blurCount += 1 }

    func flush() async -> EditorContent? {
        flushCount += 1
        return answersFlush ? EditorContent(content: content, mode: mode, revision: revision) : nil
    }

    /// A key the app let through to the editor: a character is typed,
    /// Return makes a line; any other key is only recorded.
    func key(_ chars: String) {
        keys.append(chars)
        if chars == "\u{19}" {
            onLeave?()
        } else if chars == "\r" {
            insert("\n")
        } else if let scalar = chars.unicodeScalars.first, chars.unicodeScalars.count == 1,
                  scalar.value >= 0x20, !(0xF700...0xF8FF).contains(scalar.value), scalar.value != 0x7F {
            insert(chars)
        }
    }

    /// Another app (Writing Tools, a grammar checker) changed the text, or
    /// a click moved the cursor: the editor reports it.
    func userTyped(_ text: String, at offset: Int? = nil) {
        if let offset { cursor = offset }
        insert(text)
    }

    /// Says what is under the cursor, as the editor's `state` does.
    func post(_ s: EditorStateView) {
        state = s
        model?.editorState = s
    }

    private func insert(_ text: String) {
        let s = content as NSString
        let at = min(cursor, s.length)
        content = s.replacingCharacters(in: NSRange(location: at, length: 0), with: text)
        cursor = at + text.utf16.count
        revision += 1
        report()
    }

    private func report() {
        guard reportsChanges else { return }
        model?.editorChanged(EditorContent(content: content, mode: mode, revision: revision))
    }

    /// A text as the editor would make it formatted: a paragraph per line.
    static func html(_ text: String) -> String {
        text.components(separatedBy: "\n").map { $0.isEmpty ? "<p></p>" : "<p>\($0)</p>" }.joined()
    }
}
