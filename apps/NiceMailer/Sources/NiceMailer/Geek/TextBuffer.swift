import Foundation

/// A text of several lines being edited, with its insertion point: the
/// body of a reply (`specifications/reponse.md`, section 6).
///
/// The multi-line sibling of `LineEdit`, for the same reason: the geek
/// theme reads every key itself (`GeekController.handle`), so that no key
/// is ever eaten by a view holding the focus, and so that every editing
/// rule is a pure function, tested on its own (`TextBufferTests`). The
/// view only draws the text and the caret (`ComposerTextView`).
///
/// Positions are counted in `Character`s, what the user sees as one letter
/// (an "é", an emoji of several code points): one arrow press always moves
/// over one visible letter.
///
/// # Selection
///
/// As in every macOS text view, a move with Shift selects: the *anchor*
/// stays where the selection began, the cursor moves (`extend`). Typing or
/// pasting replaces the selection, a delete key removes it, and a move
/// without Shift ends it (← and → at its start or end, `collapse`).
///
/// Swift note: `struct` is a value type; the methods that change it are
/// `mutating`.
struct TextBuffer: Equatable {
    /// The text. Lines end with `\n`, never `\r`.
    private(set) var text: String
    /// Where the next typed character goes, from 0 to `text.count`.
    private(set) var cursor: Int
    /// The column ↑ and ↓ aim for: kept while moving vertically, so that
    /// going through a short line comes back to the same column after it,
    /// as in every text editor. `nil` after any other move.
    private(set) var goalColumn: Int?
    /// Where the selection began; `nil` (or the cursor itself): nothing
    /// selected.
    private(set) var anchor: Int?

    /// Longest text kept: a reply quoting 1,000 lines stays far below; a
    /// paste of a whole book by mistake stops there.
    static let maxLength = 1_000_000

    init(_ text: String = "", cursor: Int = 0) {
        self.text = Self.clean(text)
        self.cursor = min(max(0, cursor), self.text.count)
        goalColumn = nil
        anchor = nil
    }

    var isEmpty: Bool { text.isEmpty }

    /// The lines of the text, an empty last one after a final line break.
    var lines: [Substring] { text.split(separator: "\n", omittingEmptySubsequences: false) }

    /// Line and column of the cursor, from 0.
    var position: (line: Int, column: Int) {
        let before = text.prefix(cursor)
        let line = before.reduce(0) { $1 == "\n" ? $0 + 1 : $0 }
        let column = before.count - (before.lastIndex(of: "\n").map { before.distance(from: before.startIndex, to: $0) + 1 } ?? 0)
        return (line, column)
    }

    /// The cursor in UTF-16 units, what AppKit's text views count in.
    var utf16Cursor: Int { text.prefix(cursor).utf16.count }

    /// The characters selected, from the first to after the last; `nil`
    /// when nothing is.
    var selection: Range<Int>? {
        guard let a = anchor, a != cursor else { return nil }
        return min(a, cursor)..<max(a, cursor)
    }

    /// The text selected (⌘C, ⌘X), empty when nothing is.
    var selectedText: String {
        guard let s = selection else { return "" }
        return String(text[index(s.lowerBound)..<index(s.upperBound)])
    }

    /// The selection in UTF-16 units, for drawing it.
    var utf16Selection: NSRange? {
        guard let s = selection else { return nil }
        let start = text.prefix(s.lowerBound).utf16.count
        return NSRange(location: start, length: text.prefix(s.upperBound).utf16.count - start)
    }

    // MARK: Selecting

    /// A move with Shift: `move` moves the cursor, the anchor stays where
    /// the selection began (here, if nothing was selected).
    mutating func extend(_ move: (inout TextBuffer) -> Void) {
        let a = anchor ?? cursor
        move(&self)
        anchor = a == cursor ? nil : a
    }

    /// Ends the selection with the cursor at its start or its end, as ←
    /// and → do. Returns whether there was one.
    @discardableResult
    mutating func collapse(toStart: Bool) -> Bool {
        defer { anchor = nil }
        guard let s = selection else { return false }
        cursor = toStart ? s.lowerBound : s.upperBound
        goalColumn = nil
        return true
    }

    /// Ends the selection, the cursor where it is.
    mutating func clearSelection() { anchor = nil }

    /// ⌘A: the whole text.
    mutating func selectAll() {
        goalColumn = nil
        anchor = text.isEmpty ? nil : 0
        cursor = text.count
    }

    /// Removes the selected text, the cursor where it was. Returns whether
    /// something was selected.
    private mutating func deleteSelection() -> Bool {
        defer { anchor = nil }
        guard let s = selection else { return false }
        text.removeSubrange(index(s.lowerBound)..<index(s.upperBound))
        cursor = s.lowerBound
        goalColumn = nil
        return true
    }

    /// The position, in characters, of an offset in UTF-16 units (a click
    /// in the text view): the character it falls in or before.
    func position(fromUTF16 offset: Int) -> Int {
        var units = 0
        var n = 0
        for c in text {
            if units >= offset { return n }
            units += c.utf16.count
            n += 1
        }
        return n
    }

    // MARK: Editing

    /// Inserts text at the cursor, which moves after it. Typed keys and
    /// pasted text come here, cleaned the same way.
    mutating func insert(_ s: String) {
        let piece = Self.clean(s)
        guard !piece.isEmpty else { return }
        // Typing replaces the selection.
        _ = deleteSelection()
        let room = Self.maxLength - text.count
        guard room > 0 else { return }
        let kept = piece.count > room ? String(piece.prefix(room)) : piece
        text.insert(contentsOf: kept, at: index(cursor))
        cursor += kept.count
        goalColumn = nil
    }

    /// Return: a new line.
    mutating func newline() { insert("\n") }

    /// Delete (⌫): the character left of the cursor.
    mutating func deleteBackward() {
        if deleteSelection() { return }
        goalColumn = nil
        guard cursor > 0 else { return }
        text.remove(at: index(cursor - 1))
        cursor -= 1
    }

    /// Forward delete (⌦): the character right of the cursor.
    mutating func deleteForward() {
        if deleteSelection() { return }
        goalColumn = nil
        guard cursor < text.count else { return }
        text.remove(at: index(cursor))
    }

    /// ⌥⌫: the word left of the cursor, and the spaces after it. Stops at
    /// the start of the line, as in every macOS text view.
    mutating func deleteWordBackward() {
        if deleteSelection() { return }
        goalColumn = nil
        let line = lineStart(of: cursor)
        // At the start of a line: the line break before it, as ⌫.
        guard cursor > line else { return deleteBackward() }
        let start = max(wordStart(before: cursor), line)
        text.removeSubrange(index(start)..<index(cursor))
        cursor = start
    }

    /// ⌘⌫: everything left of the cursor on its line; at the start of a
    /// line, the line break before it.
    mutating func deleteToLineStart() {
        if deleteSelection() { return }
        goalColumn = nil
        let start = lineStart(of: cursor)
        if start == cursor {
            deleteBackward()
            return
        }
        text.removeSubrange(index(start)..<index(cursor))
        cursor = start
    }

    // MARK: Moving

    mutating func moveLeft() { goalColumn = nil; cursor = max(0, cursor - 1) }
    mutating func moveRight() { goalColumn = nil; cursor = min(text.count, cursor + 1) }
    /// Home, ⌘←, `ctrl-a`: the start of the line.
    mutating func moveToLineStart() { goalColumn = nil; cursor = lineStart(of: cursor) }
    /// End, ⌘→, `ctrl-e`: the end of the line.
    mutating func moveToLineEnd() { goalColumn = nil; cursor = lineEnd(of: cursor) }
    /// ⌘↑: the start of the text.
    mutating func moveToStart() { goalColumn = nil; cursor = 0 }
    /// ⌘↓: the end of the text.
    mutating func moveToEnd() { goalColumn = nil; cursor = text.count }

    /// ↑ (`delta` -1) and ↓ (+1), by lines of the text: to the same column
    /// of the line above or below, or its end if shorter. On the first
    /// line ↑ goes to the start, on the last ↓ to the end.
    mutating func moveVertically(_ delta: Int) {
        let (line, column) = position
        let goal = goalColumn ?? column
        let all = lines
        let target = line + delta
        if target < 0 {
            cursor = 0
        } else if target >= all.count {
            cursor = text.count
        } else {
            var offset = 0
            for l in all[..<target] { offset += l.count + 1 }
            cursor = offset + min(goal, all[target].count)
        }
        goalColumn = goal
    }

    /// Moves to `position` (a click in the text, or a vertical move worked
    /// out by the view from the lines as drawn), keeping `goal` as the
    /// column to aim for.
    mutating func moveTo(_ position: Int, goal: Int? = nil) {
        cursor = min(max(0, position), text.count)
        goalColumn = goal
    }

    /// ⌥←: the start of the word left of the cursor.
    mutating func moveWordLeft() { goalColumn = nil; cursor = wordStart(before: cursor) }

    /// ⌥→: the end of the word right of the cursor.
    mutating func moveWordRight() {
        goalColumn = nil
        let chars = Array(text)
        var i = cursor
        while i < chars.count, !Self.isWordCharacter(chars[i]) { i += 1 }
        while i < chars.count, Self.isWordCharacter(chars[i]) { i += 1 }
        cursor = i
    }

    // MARK: Rules

    /// Text fit for the body: `\r\n` and `\r` become `\n`, tabs stay,
    /// other control characters and the function keys' characters go
    /// (`LineEdit.isTypable`).
    static func clean(_ s: String) -> String {
        let unified = s.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n")
        guard unified.unicodeScalars.contains(where: { !allowed($0) }) else { return unified }
        var out = String.UnicodeScalarView()
        for u in unified.unicodeScalars where allowed(u) { out.append(u) }
        return String(out)
    }

    private static func allowed(_ u: Unicode.Scalar) -> Bool {
        u == "\n" || u == "\t" || (!CharacterSet.controlCharacters.contains(u) && !(0xF700...0xF8FF).contains(u.value))
    }

    // MARK: Helpers

    private func index(_ n: Int) -> String.Index { text.index(text.startIndex, offsetBy: n) }

    /// Start of the line holding `position`.
    func lineStart(of position: Int) -> Int {
        let before = text.prefix(position)
        guard let nl = before.lastIndex(of: "\n") else { return 0 }
        return before.distance(from: before.startIndex, to: nl) + 1
    }

    /// End of the line holding `position` (before its line break).
    func lineEnd(of position: Int) -> Int {
        let after = text.dropFirst(position)
        guard let nl = after.firstIndex(of: "\n") else { return text.count }
        return position + after.distance(from: after.startIndex, to: nl)
    }

    private func wordStart(before position: Int) -> Int {
        let chars = Array(text.prefix(position))
        var i = position
        while i > 0, !Self.isWordCharacter(chars[i - 1]) { i -= 1 }
        while i > 0, Self.isWordCharacter(chars[i - 1]) { i -= 1 }
        return i
    }

    private static func isWordCharacter(_ c: Character) -> Bool { c.isLetter || c.isNumber }
}
