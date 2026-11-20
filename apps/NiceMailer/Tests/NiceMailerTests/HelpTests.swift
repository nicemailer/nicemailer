import Foundation
import Testing
@testable import NiceMailer

/// The list of keys (`?`, `Help`): its content, and how its groups are
/// split into columns for the window's width.
struct HelpTests {
    /// A group of `n` keys, for the layout tests.
    func group(_ title: String, _ n: Int, label: String = "label") -> HelpGroup {
        HelpGroup(title: title, entries: (0..<n).map { HelpEntry(key: "k\($0)", label: label) })
    }

    // MARK: Content

    @Test func oneLinePerKeyAndNoKeyTwice() {
        let keys = Help.groups().flatMap(\.entries).map(\.key)
        #expect(Set(keys).count == keys.count)
        // u and U are two lines: the toggle, and marking read.
        #expect(keys.contains("u") && keys.contains("U"))
        #expect(!keys.contains { $0.contains("*") && $0 != "*" })
        #expect(Help.groups().allSatisfy { !$0.entries.isEmpty })
    }

    /// The account picker has its line with several accounts only.
    @Test func theAccountPickerLineNeedsSeveralAccounts() {
        let several = Help.groups().flatMap(\.entries)
        #expect(several.contains { $0.key == "@" && $0.label == L("go to an account") })
        let single = Help.groups(severalAccounts: false).flatMap(\.entries)
        #expect(!single.contains { $0.key == "@" })
        // Nothing else changes.
        #expect(single.count == several.count - 1)
    }

    @Test func whatIsNotThereYetIsMarkedSo() {
        let entries = Help.groups().flatMap(\.entries)
        #expect(entries.first { $0.key == "r" }?.notYet == false)
        for key in ["R", "f", "c"] {
            #expect(entries.first { $0.key == key }?.notYet == false, "\(key)")
        }
    }

    // MARK: Columns

    @Test func columnsKeepTheOrderAndNeverCutAGroup() {
        let groups = Help.groups()
        for count in 1...4 {
            let columns = Help.columns(groups, count: count)
            #expect(columns.count == min(count, groups.count))
            #expect(columns.flatMap { $0 } == groups)
            #expect(columns.allSatisfy { !$0.isEmpty })
        }
    }

    @Test func theTallestColumnIsAsShortAsPossible() {
        // Heights 5, 5, 2, 2, 2 (keys + title): in two columns, 5+1+5 = 11
        // against 2+1+2+1+2 = 8 would leave 11; 5 | 5,2,2,2 gives 5 and 14;
        // the best is 5,5 | 2,2,2: 11 and 8.
        let groups = [group("a", 4), group("b", 4), group("c", 1), group("d", 1), group("e", 1)]
        let columns = Help.columns(groups, count: 2)
        #expect(columns.map { $0.map(\.title) } == [["a", "b"], ["c", "d", "e"]])
        #expect(columns.map(Help.height) == [11, 8])
    }

    @Test func onATieTheFirstColumnsAreTheFullerOnes() {
        let groups = [group("a", 1), group("b", 1), group("c", 1)]
        // a | b, c and a, b | c are both 5 lines tall: the fuller first
        // column is kept.
        let columns = Help.columns(groups, count: 2)
        #expect(columns.map { $0.map(\.title) } == [["a", "b"], ["c"]])
    }

    @Test func aColumnSharesOneKeyWidthAndIsAsWideAsItsLongestLine() {
        let column = [
            HelpGroup(title: "A long title of a group", entries: [HelpEntry(key: "x", label: "ab")]),
            HelpGroup(title: "B", entries: [HelpEntry(key: "⌘+ ⌘−", label: "abc")]),
        ]
        #expect(Help.keyWidth(column) == 5)
        // max(title 23, key 5 + gap 2 + label 3).
        #expect(Help.width(column) == 23)
        #expect(Help.width([[group("a", 1)], [group("b", 1)]]) == 2 * (2 + Help.keyGap + 5) + Help.columnGap)
    }

    @Test func theWindowWidthDecidesTheNumberOfColumns() {
        let groups = Help.groups()
        let char: CGFloat = 7, padding: CGFloat = 14
        func needed(_ count: Int) -> CGFloat {
            CGFloat(Help.width(Help.columns(groups, count: count))) * char + 2 * padding
        }
        #expect(Help.columnCount(groups, available: needed(3), charWidth: char, padding: padding) == 3)
        #expect(Help.columnCount(groups, available: 5000, charWidth: char, padding: padding) == 3)
        #expect(Help.columnCount(groups, available: needed(3) - 1, charWidth: char, padding: padding) == 2)
        #expect(Help.columnCount(groups, available: needed(2) - 1, charWidth: char, padding: padding) == 1)
        // Too narrow even for one: one column all the same.
        #expect(Help.columnCount(groups, available: 10, charWidth: char, padding: padding) == 1)
    }

    /// The real help, in three columns, is far shorter than in one: the
    /// point of the layout.
    @Test func threeColumnsAreAboutAThirdOfOne() {
        let one = Help.height(Help.groups())
        let three = Help.columns(Help.groups(), count: 3).map(Help.height).max() ?? 0
        #expect(three * 2 < one)
    }
}
