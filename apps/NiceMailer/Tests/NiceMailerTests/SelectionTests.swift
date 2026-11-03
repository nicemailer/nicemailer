import Testing
@testable import NiceMailer

/// Selection rules of the mail list (`Selection`), Finder-style.
struct SelectionTests {
    let ids = [10, 20, 30, 40, 50]

    @Test func plainClickSelectsAndClearsTheMarks() {
        let r = Selection.click(on: 30, in: ids, cursor: 10, marked: [20, 40], command: false, shift: false)
        #expect(r == Selection.Result(cursor: 30, marked: []))
    }

    @Test func commandClickAddsTheCurrentMailThenToggles() {
        let r1 = Selection.click(on: 40, in: ids, cursor: 20, marked: [], command: true, shift: false)
        #expect(r1 == Selection.Result(cursor: 40, marked: [20, 40]))
        let r2 = Selection.click(on: 40, in: ids, cursor: 40, marked: r1.marked, command: true, shift: false)
        #expect(r2.marked == [20])
    }

    @Test func commandClickOnTheCurrentMailOnlyMarksIt() {
        let r = Selection.click(on: 20, in: ids, cursor: 20, marked: [], command: true, shift: false)
        #expect(r.marked == [20])
    }

    @Test func shiftClickMarksTheRangeInBothDirections() {
        let down = Selection.click(on: 40, in: ids, cursor: 20, marked: [], command: false, shift: true)
        #expect(down == Selection.Result(cursor: 40, marked: [20, 30, 40]))
        let up = Selection.click(on: 10, in: ids, cursor: 30, marked: [50], command: false, shift: true)
        #expect(up.marked == [10, 20, 30, 50])
    }

    @Test func shiftClickWithoutCursorIsAPlainClick() {
        let r = Selection.click(on: 30, in: ids, cursor: nil, marked: [], command: false, shift: true)
        #expect(r == Selection.Result(cursor: 30, marked: []))
    }

    @Test func markAndMoveMarksBothMailsAndStaysInTheList() {
        let r = Selection.markAndMove(1, in: ids, cursor: 20, marked: [])
        #expect(r == Selection.Result(cursor: 30, marked: [20, 30]))
        let end = Selection.markAndMove(1, in: ids, cursor: 50, marked: [])
        #expect(end == Selection.Result(cursor: 50, marked: [50]))
        let up = Selection.markAndMove(-1, in: ids, cursor: 10, marked: [])
        #expect(up.cursor == 10)
    }
}
