import Foundation
import Testing
@testable import NiceMailer

/// The editing of the reply's text (`TextBuffer`), as pure functions.
struct TextBufferTests {
    @Test func typingGoesAtTheCursor() {
        var b = TextBuffer("\n\n> Hello", cursor: 0)
        b.insert("Merci")
        #expect(b.text == "Merci\n\n> Hello")
        #expect(b.cursor == 5)
        b.newline()
        b.insert("Bonne journée")
        #expect(b.text == "Merci\nBonne journée\n\n> Hello")
        #expect(b.position == (1, 13))
    }

    @Test func pastedTextKeepsItsLinesButNotItsControlCharacters() {
        var b = TextBuffer()
        b.insert("a\r\nb\rc\u{7}\u{F700}\td")
        #expect(b.text == "a\nb\nc\td")
        #expect(b.cursor == 7)
    }

    @Test func deletingOnBothSides() {
        var b = TextBuffer("abc\ndef", cursor: 4)
        b.deleteBackward()
        #expect(b.text == "abcdef")
        #expect(b.cursor == 3)
        b.deleteForward()
        #expect(b.text == "abcef")
        b.moveToStart()
        b.deleteBackward()
        #expect(b.text == "abcef")
        b.moveToEnd()
        b.deleteForward()
        #expect(b.text == "abcef")
    }

    @Test func deletingAWordStopsAtTheLine() {
        var b = TextBuffer("Bonjour à tous\nMerci beaucoup", cursor: 29)
        b.deleteWordBackward()
        #expect(b.text == "Bonjour à tous\nMerci ")
        b.deleteWordBackward()
        #expect(b.text == "Bonjour à tous\n")
        // At the start of a line: the line break, as ⌫.
        b.deleteWordBackward()
        #expect(b.text == "Bonjour à tous")
    }

    @Test func commandDeleteRemovesTheStartOfTheLine() {
        var b = TextBuffer("one\ntwo three", cursor: 8)
        b.deleteToLineStart()
        #expect(b.text == "one\nthree")
        #expect(b.cursor == 4)
        b.deleteToLineStart()
        #expect(b.text == "onethree")
    }

    @Test func linesStartsAndEnds() {
        var b = TextBuffer("first\nsecond\nthird", cursor: 9)
        b.moveToLineStart()
        #expect(b.cursor == 6)
        b.moveToLineEnd()
        #expect(b.cursor == 12)
        b.moveToStart()
        #expect(b.cursor == 0)
        b.moveToEnd()
        #expect(b.cursor == 18)
    }

    @Test func verticalMovesKeepTheirColumn() {
        // Column 5 of a long line, through a short one, back to column 5.
        var b = TextBuffer("abcdefgh\nab\nabcdefgh", cursor: 5)
        b.moveVertically(1)
        #expect(b.position == (1, 2))
        b.moveVertically(1)
        #expect(b.position == (2, 5))
        b.moveVertically(-1)
        b.moveVertically(-1)
        #expect(b.position == (0, 5))
        // Above the first line: the start; below the last: the end.
        b.moveVertically(-1)
        #expect(b.cursor == 0)
        b.moveTo(3)
        b.moveVertically(5)
        #expect(b.cursor == b.text.count)
        // Another move forgets the column.
        b.moveLeft()
        #expect(b.goalColumn == nil)
    }

    @Test func wordsAreLettersAndDigits() {
        var b = TextBuffer("see alice@example.org now", cursor: 0)
        b.moveWordRight()
        #expect(b.cursor == 3)
        b.moveWordRight()
        #expect(b.cursor == 9)
        b.moveWordLeft()
        #expect(b.cursor == 4)
    }

    @Test func utf16PositionsForAppKit() {
        // "é" is one UTF-16 unit, the emoji two.
        let b = TextBuffer("é😀x", cursor: 2)
        #expect(b.utf16Cursor == 3)
        #expect(b.position(fromUTF16: 0) == 0)
        #expect(b.position(fromUTF16: 1) == 1)
        #expect(b.position(fromUTF16: 3) == 2)
        #expect(b.position(fromUTF16: 4) == 3)
        #expect(b.position(fromUTF16: 99) == 3)
    }

    @Test func oneArrowIsOneVisibleLetter() {
        var b = TextBuffer("a👍🏽b", cursor: 1)
        b.moveRight()
        #expect(b.cursor == 2)
        b.deleteBackward()
        #expect(b.text == "ab")
    }

    @Test func theCursorStaysInTheText() {
        let b = TextBuffer("abc", cursor: 99)
        #expect(b.cursor == 3)
        var c = TextBuffer("abc")
        c.moveTo(-4)
        #expect(c.cursor == 0)
    }

    // MARK: Selection

    @Test func shiftMovesSelectFromWhereTheSelectionBegan() {
        var b = TextBuffer("Bonjour Alice", cursor: 8)
        b.extend { $0.moveWordRight() }
        #expect(b.selection == 8..<13)
        #expect(b.selectedText == "Alice")
        // Back over the anchor: the selection goes to the other side.
        b.extend { $0.moveToLineStart() }
        #expect(b.selection == 0..<8)
        #expect(b.selectedText == "Bonjour ")
        // Back to the anchor: nothing selected.
        b.extend { $0.moveTo(8) }
        #expect(b.selection == nil)
    }

    @Test func aMoveWithoutShiftEndsTheSelectionAtItsSide() {
        var b = TextBuffer("abcdef", cursor: 1)
        b.extend { $0.moveTo(4) }
        let collapsed = b.collapse(toStart: true)
        #expect(collapsed)
        #expect(b.cursor == 1)
        #expect(b.selection == nil)
        b.extend { $0.moveTo(4) }
        b.collapse(toStart: false)
        #expect(b.cursor == 4)
        let again = b.collapse(toStart: true)
        #expect(!again)
        #expect(b.cursor == 4)
    }

    @Test func typingAndDeletingReplaceTheSelection() {
        var b = TextBuffer("Bonjour Alice,", cursor: 8)
        b.extend { $0.moveWordRight() }
        b.insert("Bob")
        #expect(b.text == "Bonjour Bob,")
        #expect(b.cursor == 11)
        #expect(b.selection == nil)
        let deletes: [(inout TextBuffer) -> Void] = [
            { $0.deleteBackward() }, { $0.deleteForward() }, { $0.deleteWordBackward() }, { $0.deleteToLineStart() },
        ]
        for delete in deletes {
            var d = TextBuffer("one two three", cursor: 4)
            d.extend { $0.moveWordRight() }
            delete(&d)
            #expect(d.text == "one  three")
            #expect(d.cursor == 4)
        }
    }

    @Test func selectAllAndAcrossLines() {
        var b = TextBuffer("ab\ncd", cursor: 1)
        b.selectAll()
        #expect(b.selectedText == "ab\ncd")
        #expect(b.utf16Selection == NSRange(location: 0, length: 5))
        var e = TextBuffer("")
        e.selectAll()
        #expect(e.selection == nil)
        // The selection in UTF-16 units: an emoji counts two.
        var u = TextBuffer("😀ab", cursor: 0)
        u.extend { $0.moveRight() }
        #expect(u.utf16Selection == NSRange(location: 0, length: 2))
    }

    @Test func optionUpAndDownGoOnByParagraph() {
        // ⌥↓ goes to the end of the paragraph, then of the next one at each
        // press, as in macOS: ⇧⌥↓ thus selects paragraph after paragraph.
        var b = TextBuffer("un\ndeux\n\ntrois")
        b.moveTo(0)
        b.extend { $0.moveToParagraphEnd() }
        #expect(b.selectedText == "un")
        b.extend { $0.moveToParagraphEnd() }
        #expect(b.selectedText == "un\ndeux")
        b.extend { $0.moveToParagraphEnd() }
        #expect(b.selectedText == "un\ndeux\n", "the empty line is a paragraph")
        b.extend { $0.moveToParagraphEnd() }
        b.extend { $0.moveToParagraphEnd() }
        #expect(b.selectedText == b.text, "it stops at the end")
        // ⌥↑ the same way back, from inside a paragraph to its start first.
        var u = TextBuffer("un\ndeux")
        u.moveTo(6)
        u.moveToParagraphStart()
        #expect(u.cursor == 3)
        u.moveToParagraphStart()
        #expect(u.cursor == 0)
        u.moveToParagraphStart()
        #expect(u.cursor == 0)
    }
}
