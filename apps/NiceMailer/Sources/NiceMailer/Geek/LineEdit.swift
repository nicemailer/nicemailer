import Foundation

/// A one-line text being edited, with its insertion point: the text typed
/// in a picker (`F`, `/`, `@`, `t`, `g`, `l`) or in the contacts filter.
///
/// The geek theme reads every key itself (`GeekController.handle`) instead
/// of giving the focus to a `TextField`, so that no key is ever eaten by a
/// field. The price is that the editing a text field gives for free has to
/// be done here: moving the cursor with the arrows, deleting on either side
/// of it, pasting. Keeping it in a small value type with no AppKit in it
/// makes every rule a pure function, tested on its own (`LineEditTests`).
///
/// Swift note: a `struct` is a value type. `var q = line; q.insert("a")`
/// changes the copy only, and a method that changes the value must be
/// marked `mutating`.
struct LineEdit: Equatable {
    /// The text, never containing a line break or a control character.
    private(set) var text: String
    /// Where the next typed character goes, in characters from the start
    /// (0: before the first one, `text.count`: at the end).
    ///
    /// Counted in `Character`s (what the user sees as one letter, an "é"
    /// or an emoji made of several code points), so that one arrow press
    /// always moves over one visible letter.
    private(set) var cursor: Int
    /// The whole text is selected, as in a macOS field reached with Tab:
    /// typing or pasting replaces it, Delete clears it, and an arrow ends
    /// the selection (← at the start, → at the end) to edit it instead.
    /// A value panel opens this way on the current value of its attribute
    /// (Subject…, Text…): what is typed replaces it, and the old value is
    /// seen rather than silently kept.
    private(set) var selectedAll: Bool

    /// Longest text kept: a picker filters a list, and a paste of a whole
    /// mail by mistake should not slow every key down.
    static let maxLength = 1_000

    /// A text with the cursor at its end, where typing goes on, or with
    /// the whole text selected (`selectedAll`).
    init(_ text: String = "", selected: Bool = false) {
        self.text = Self.clean(text)
        cursor = self.text.count
        selectedAll = selected && !self.text.isEmpty
    }

    var isEmpty: Bool { text.isEmpty }
    /// The text left of the cursor, and the text right of it (for drawing
    /// the caret between them).
    var beforeCursor: String { String(text.prefix(cursor)) }
    var afterCursor: String { String(text.dropFirst(cursor)) }

    // MARK: Editing

    /// Inserts text at the cursor, which moves after it. Typed keys and
    /// pasted text both come here, cleaned the same way (`clean`).
    mutating func insert(_ s: String) {
        let piece = Self.clean(s)
        guard !piece.isEmpty else { return }
        if selectedAll { clear() }
        let room = Self.maxLength - text.count
        guard room > 0 else { return }
        let kept = String(piece.prefix(room))
        let i = index(cursor)
        text.insert(contentsOf: kept, at: i)
        cursor += kept.count
    }

    /// Delete (⌫): removes the character left of the cursor.
    mutating func deleteBackward() {
        if selectedAll { clear(); return }
        guard cursor > 0 else { return }
        text.remove(at: index(cursor - 1))
        cursor -= 1
    }

    /// Forward delete (⌦, fn-⌫): removes the character right of the cursor.
    mutating func deleteForward() {
        if selectedAll { clear(); return }
        guard cursor < text.count else { return }
        text.remove(at: index(cursor))
    }

    /// ⌥⌫: removes the word left of the cursor (and the spaces after it),
    /// as in every macOS text field.
    mutating func deleteWordBackward() {
        if selectedAll { clear(); return }
        let start = wordStart(before: cursor)
        text.removeSubrange(index(start)..<index(cursor))
        cursor = start
    }

    /// ⌘⌫: removes everything left of the cursor.
    mutating func deleteToStart() {
        if selectedAll { clear(); return }
        text.removeSubrange(text.startIndex..<index(cursor))
        cursor = 0
    }

    // MARK: Moving

    // A move with the whole text selected ends the selection at the side
    // the arrow points to, as in every macOS field: ← puts the cursor
    // before the text, → after it.

    mutating func moveLeft() {
        if endSelection(atStart: true) { return }
        cursor = max(0, cursor - 1)
    }
    mutating func moveRight() {
        if endSelection(atStart: false) { return }
        cursor = min(text.count, cursor + 1)
    }
    mutating func moveToStart() { selectedAll = false; cursor = 0 }
    mutating func moveToEnd() { selectedAll = false; cursor = text.count }
    /// ⌥←: to the start of the word left of the cursor.
    mutating func moveWordLeft() {
        if endSelection(atStart: true) { return }
        cursor = wordStart(before: cursor)
    }
    /// ⌥→: to the end of the word right of the cursor.
    mutating func moveWordRight() {
        if endSelection(atStart: false) { return }
        let chars = Array(text)
        var i = cursor
        while i < chars.count, !Self.isWordCharacter(chars[i]) { i += 1 }
        while i < chars.count, Self.isWordCharacter(chars[i]) { i += 1 }
        cursor = i
    }

    // MARK: Rules

    /// Whether a key's text is something to type, rather than a key with
    /// no character of its own.
    ///
    /// AppKit gives the arrows, Home, End, F1… characters of the Unicode
    /// private use area (U+F700 to U+F8FF, `NSUpArrowFunctionKey`…). They
    /// are not control characters, so testing those alone let the left
    /// arrow add an invisible character to the text instead of moving.
    static func isTypable(_ key: String) -> Bool {
        !key.isEmpty && key.unicodeScalars.allSatisfy { s in
            !CharacterSet.controlCharacters.contains(s) && !(0xF700...0xF8FF).contains(s.value)
        }
    }

    /// Text made fit for one line: what is pasted from another app often
    /// ends with a line break, or spreads an address over two lines. Line
    /// breaks and tabs become one space each run, other control and
    /// function-key characters go, and the whole is cut at `maxLength`.
    /// Spaces inside are kept: they separate the words of a search.
    static func clean(_ s: String) -> String {
        var out = ""
        var pendingSpace = false
        for c in s {
            if c.isNewline || c == "\t" {
                pendingSpace = true
                continue
            }
            guard isTypable(String(c)) else { continue }
            if pendingSpace, !out.isEmpty, out.last != " ", c != " " { out.append(" ") }
            pendingSpace = false
            out.append(c)
            if out.count >= maxLength { break }
        }
        return out
    }

    /// A pasted text, cleaned (`clean`) and without the spaces around it:
    /// copying a word by double-click in Mail or a browser often takes the
    /// space after it too.
    static func pasted(_ s: String) -> String {
        clean(s).trimmingCharacters(in: .whitespaces)
    }

    // MARK: Helpers

    /// Empties the text (a selected text replaced or deleted).
    private mutating func clear() {
        text = ""
        cursor = 0
        selectedAll = false
    }

    /// Ends a whole-text selection, the cursor at one end. Returns whether
    /// there was one (the move is then done).
    private mutating func endSelection(atStart: Bool) -> Bool {
        guard selectedAll else { return false }
        selectedAll = false
        cursor = atStart ? 0 : text.count
        return true
    }

    /// Index of the `n`-th character.
    /// Swift note: a `String` is not indexed by integers, since a character
    /// can take several bytes; `String.Index` values are walked to instead.
    private func index(_ n: Int) -> String.Index {
        text.index(text.startIndex, offsetBy: n)
    }

    /// Start of the word that ends at, or before, `position`.
    private func wordStart(before position: Int) -> Int {
        let chars = Array(text)
        var i = position
        while i > 0, !Self.isWordCharacter(chars[i - 1]) { i -= 1 }
        while i > 0, Self.isWordCharacter(chars[i - 1]) { i -= 1 }
        return i
    }

    /// Letters and digits make words; spaces and punctuation (`@`, `.`,
    /// `-`) end them, so ⌥← stops inside an address as in Mail.
    private static func isWordCharacter(_ c: Character) -> Bool {
        c.isLetter || c.isNumber
    }
}
