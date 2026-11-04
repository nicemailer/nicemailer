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
}
