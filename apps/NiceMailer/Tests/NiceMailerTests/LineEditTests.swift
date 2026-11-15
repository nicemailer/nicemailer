import Testing
@testable import NiceMailer

/// Editing of a typed line (`LineEdit`): the cursor, deleting on both
/// sides, words, and what a paste becomes.
struct LineEditTests {
    @Test func aNewLineHasItsCursorAtTheEnd() {
        let line = LineEdit("abc")
        #expect(line.cursor == 3)
        #expect(line.beforeCursor == "abc" && line.afterCursor == "")
    }

    @Test func theArrowsMoveTheCursorAndTypingInsertsThere() {
        var line = LineEdit("alce")
        line.moveLeft(); line.moveLeft()
        #expect(line.cursor == 2)
        line.insert("i")
        #expect(line.text == "alice" && line.cursor == 3)
        line.moveRight(); line.moveRight(); line.moveRight()
        #expect(line.cursor == 5)
        line.moveToStart()
        line.moveLeft()
        #expect(line.cursor == 0)
    }

    @Test func deleteRemovesLeftAndForwardDeleteRight() {
        var line = LineEdit("abcd")
        line.moveLeft(); line.moveLeft()
        line.deleteBackward()
        #expect(line.text == "acd" && line.cursor == 1)
        line.deleteForward()
        #expect(line.text == "ad" && line.cursor == 1)
        line.moveToStart(); line.deleteBackward()
        line.moveToEnd(); line.deleteForward()
        #expect(line.text == "ad")
    }

    @Test func aCharacterMadeOfSeveralCodePointsIsOneStep() {
        // "é" as e + combining accent, and a flag: one letter each.
        var line = LineEdit("e\u{301}🇫🇷x")
        #expect(line.cursor == 3)
        line.moveLeft(); line.deleteBackward()
        #expect(line.text == "e\u{301}x")
    }

    @Test func wordsStopAtSpacesAndPunctuation() {
        var line = LineEdit("jean.dupont@acme.fr devis")
        line.moveWordLeft()
        #expect(line.afterCursor == "devis")
        line.moveWordLeft()
        #expect(line.afterCursor == "fr devis")
        line.moveWordRight()
        #expect(line.beforeCursor == "jean.dupont@acme.fr")
        line.moveToEnd()
        line.deleteWordBackward()
        #expect(line.text == "jean.dupont@acme.fr ")
        line.moveLeft(); line.moveLeft()
        line.deleteToStart()
        #expect(line.text == "r " && line.cursor == 0)
    }

    @Test func functionKeyCharactersAreNotTyped() {
        // The left arrow's character used to be added to the text.
        #expect(!LineEdit.isTypable("\u{F702}"))
        #expect(!LineEdit.isTypable("\u{1b}"))
        #expect(!LineEdit.isTypable(""))
        #expect(LineEdit.isTypable("é") && LineEdit.isTypable(" ") && LineEdit.isTypable("@"))
        var line = LineEdit("a")
        line.insert("\u{F702}")
        #expect(line.text == "a")
    }

    @Test func aPasteBecomesOneTrimmedLine() {
        #expect(LineEdit.pasted("  alice@acme.fr\n") == "alice@acme.fr")
        #expect(LineEdit.pasted("Jean Dupont\r\n<jean@acme.fr>") == "Jean Dupont <jean@acme.fr>")
        #expect(LineEdit.pasted("a\t\tb") == "a b")
        #expect(LineEdit.pasted("\n\n") == "")
        // Spaces inside are the words of a search: kept.
        #expect(LineEdit.pasted("devis  toiture") == "devis  toiture")
    }

    @Test func aSelectedTextIsReplacedByTypingAndClearedByDelete() {
        var line = LineEdit("devis", selected: true)
        #expect(line.selectedAll && line.cursor == 5)
        line.insert("f")
        #expect(line.text == "f" && line.cursor == 1 && !line.selectedAll)
        line = LineEdit("devis", selected: true)
        line.deleteBackward()
        #expect(line.text == "" && !line.selectedAll)
        line = LineEdit("devis", selected: true)
        line.deleteForward()
        #expect(line.text == "")
        // An empty text has nothing to select.
        #expect(!LineEdit("", selected: true).selectedAll)
    }

    @Test func anArrowEndsTheSelectionAtItsSide() {
        var line = LineEdit("devis", selected: true)
        line.moveLeft()
        #expect(!line.selectedAll && line.cursor == 0)
        line.insert("x")
        #expect(line.text == "xdevis")
        line = LineEdit("devis", selected: true)
        line.moveRight()
        #expect(!line.selectedAll && line.cursor == 5)
        line.insert(" -x")
        #expect(line.text == "devis -x")
        line = LineEdit("a b", selected: true)
        line.moveWordLeft()
        #expect(line.cursor == 0 && line.text == "a b")
        line = LineEdit("a b", selected: true)
        line.moveToEnd()
        #expect(line.cursor == 3 && !line.selectedAll)
    }

    @Test func theTextIsCappedInLength() {
        var line = LineEdit(String(repeating: "x", count: 1_500))
        #expect(line.text.count == LineEdit.maxLength)
        line.insert("y")
        #expect(line.text.count == LineEdit.maxLength && !line.text.contains("y"))
    }

    @Test func shiftMovesSelectAPartAsInAMacField() {
        // ⇧⌥←: the word before the cursor; again: the one before it too.
        var line = LineEdit("Re: devis toiture")
        line.extend { $0.moveWordLeft() }
        #expect(line.selectedText == "toiture" && line.selection == 10..<17)
        line.extend { $0.moveWordLeft() }
        #expect(line.selectedText == "devis toiture")
        // ⇧→ shrinks it from the cursor's side, the anchor staying put.
        line.extend { $0.moveRight() }
        #expect(line.selectedText == "evis toiture")
        // Back over the anchor: nothing selected.
        var back = LineEdit("abc")
        back.extend { $0.moveLeft() }
        back.extend { $0.moveRight() }
        #expect(back.selection == nil && back.cursor == 3)
        // Typing replaces the part selected, the cursor after it.
        line.insert("X")
        #expect(line.text == "Re: dX" && line.cursor == 6 && line.selection == nil)
        // ⌫ removes it; ⌥⌫ too, and no more than it.
        line = LineEdit("un deux trois")
        line.extend { $0.moveWordLeft() }
        line.deleteWordBackward()
        #expect(line.text == "un deux " && line.cursor == 8)
        // ⇧⌘←: to the start; ← ends it at its start, ⌥→ from its end.
        line = LineEdit("un deux trois")
        line.extend { $0.moveToStart() }
        #expect(line.selectedAll)
        line.moveLeft()
        #expect(line.cursor == 0 && line.selection == nil)
        line.extend { $0.moveWordRight() }
        #expect(line.selectedText == "un")
        line.moveWordRight()
        #expect(line.cursor == 7 && line.selection == nil)
    }

    @Test func commandASelectsTheWholeLine() {
        var line = LineEdit("devis")
        line.selectAll()
        #expect(line.selectedAll && line.selectedText == "devis")
        var empty = LineEdit()
        empty.selectAll()
        #expect(empty.selection == nil)
    }
}
