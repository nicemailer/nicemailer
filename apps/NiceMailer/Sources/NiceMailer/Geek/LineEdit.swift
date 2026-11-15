import Foundation

/// A one-line text being edited, with its insertion point and its
/// selection: the text typed in a picker (`F`, `/`, `@`, `t`, `g`, `l`), in
/// the contacts filter, or in a field of the reply (subject, recipients).
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
    /// Where the selection began, as in every macOS text field: the
    /// selected text runs from here to the cursor, on either side of it.
    /// `nil`, or equal to the cursor: nothing is selected. A move with
    /// Shift (⇧← ⇧⌥→ ⇧⌘←…) keeps it and moves the cursor (`extend`);
    /// typing or pasting replaces the selection, ⌫ removes it, and an
    /// arrow alone ends it (← at its start, → at its end).
    /// The same rules as the text of a reply (`TextBuffer`, "Selection").
    private(set) var anchor: Int?

    /// The whole text is selected, as in a macOS field reached with Tab or
    /// after ⌘A. A value panel opens this way on the current value of its
    /// attribute (Subject…, Text…): what is typed replaces it, and the old
    /// value is seen rather than silently kept.
    var selectedAll: Bool { !text.isEmpty && selection == 0..<text.count }

    /// The characters selected, from the first to after the last; `nil`
    /// when nothing is.
    var selection: Range<Int>? {
        guard let a = anchor, a != cursor else { return nil }
        return min(a, cursor)..<max(a, cursor)
    }

    /// The text selected (⌘C, ⌘X), empty when nothing is.
    var selectedText: String {
        guard let r = selection else { return "" }
        return String(text[index(r.lowerBound)..<index(r.upperBound)])
    }

    /// Longest text kept: a picker filters a list, and a paste of a whole
    /// mail by mistake should not slow every key down.
    static let maxLength = 1_000

    /// A text with the cursor at its end, where typing goes on, or with
    /// the whole text selected (`selectedAll`).
    init(_ text: String = "", selected: Bool = false) {
        self.text = Self.clean(text)
        cursor = self.text.count
        anchor = selected && !self.text.isEmpty ? 0 : nil
    }

    var isEmpty: Bool { text.isEmpty }
    /// The text left of the cursor, and the text right of it (for drawing
    /// the caret between them).
    var beforeCursor: String { String(text.prefix(cursor)) }
    var afterCursor: String { String(text.dropFirst(cursor)) }

    // MARK: Selecting

    /// A move with Shift: `move` moves the cursor, the anchor stays where
    /// the selection began (here, if nothing was selected).
    /// Swift note: `(inout LineEdit) -> Void` is a function that may change
    /// the value it is given; `move(&self)` hands it this very value.
    mutating func extend(_ move: (inout LineEdit) -> Void) {
        let a = anchor ?? cursor
        anchor = nil
        move(&self)
        anchor = a == cursor ? nil : a
    }

    /// ⌘A: the whole text.
    mutating func selectAll() {
        anchor = text.isEmpty ? nil : 0
        cursor = text.count
    }

    // MARK: Editing

    /// Inserts text at the cursor, which moves after it. Typed keys and
    /// pasted text both come here, cleaned the same way (`clean`).
    mutating func insert(_ s: String) {
        let piece = Self.clean(s)
        guard !piece.isEmpty else { return }
        deleteSelection()
        let room = Self.maxLength - text.count
        guard room > 0 else { return }
        let kept = String(piece.prefix(room))
        let i = index(cursor)
        text.insert(contentsOf: kept, at: i)
        cursor += kept.count
    }

    /// Delete (⌫): removes the character left of the cursor.
    mutating func deleteBackward() {
        if deleteSelection() { return }
        guard cursor > 0 else { return }
        text.remove(at: index(cursor - 1))
        cursor -= 1
    }

    /// Forward delete (⌦, fn-⌫): removes the character right of the cursor.
    mutating func deleteForward() {
        if deleteSelection() { return }
        guard cursor < text.count else { return }
        text.remove(at: index(cursor))
    }

    /// ⌥⌫: removes the word left of the cursor (and the spaces after it),
    /// as in every macOS text field.
    mutating func deleteWordBackward() {
        if deleteSelection() { return }
        let start = wordStart(before: cursor)
        text.removeSubrange(index(start)..<index(cursor))
        cursor = start
    }

    /// ⌘⌫: removes everything left of the cursor.
    mutating func deleteToStart() {
        if deleteSelection() { return }
        text.removeSubrange(text.startIndex..<index(cursor))
        cursor = 0
    }

    // MARK: Moving

    // A move with a selection ends it at the side the arrow points to, as
    // in every macOS field: ← puts the cursor at its start, → at its end;
    // ⌥← ⌥→ then go on by a word from there, ⌘← ⌘→ to the ends of the
    // line. Under `extend` (Shift), the anchor is cleared first, so these
    // only move.

    mutating func moveLeft() {
        if endSelection(atStart: true) { return }
        cursor = max(0, cursor - 1)
    }
    mutating func moveRight() {
        if endSelection(atStart: false) { return }
        cursor = min(text.count, cursor + 1)
    }
    mutating func moveToStart() { anchor = nil; cursor = 0 }
    mutating func moveToEnd() { anchor = nil; cursor = text.count }
    /// ⌥←: to the start of the word left of the cursor (of the selection).
    mutating func moveWordLeft() {
        endSelection(atStart: true)
        cursor = wordStart(before: cursor)
    }
    /// ⌥→: to the end of the word right of the cursor (of the selection).
    mutating func moveWordRight() {
        endSelection(atStart: false)
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

    /// Removes the selected text, the cursor where it began. Returns
    /// whether something was selected (the deletion is then done).
    @discardableResult
    private mutating func deleteSelection() -> Bool {
        defer { anchor = nil }
        guard let r = selection else { return false }
        text.removeSubrange(index(r.lowerBound)..<index(r.upperBound))
        cursor = r.lowerBound
        return true
    }

    /// Ends the selection, the cursor at its start or its end. Returns
    /// whether there was one (for ← →, the move is then done).
    @discardableResult
    private mutating func endSelection(atStart: Bool) -> Bool {
        defer { anchor = nil }
        guard let r = selection else { return false }
        cursor = atStart ? r.lowerBound : r.upperBound
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
