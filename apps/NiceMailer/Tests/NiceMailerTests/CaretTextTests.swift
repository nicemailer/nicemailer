import AppKit
import SwiftUI
import Testing
@testable import NiceMailer

/// The caret of a typed line (`CaretText`) takes no room: the letters sit
/// where they would without it, wherever the cursor is.
@MainActor
struct CaretTextTests {
    /// Width SwiftUI gives a view laid out on its own, in the geek font.
    ///
    /// Swift note: `NSHostingView` puts a SwiftUI view in an AppKit view;
    /// its `fittingSize` is the size the view asks for, which is what the
    /// picker header gives it.
    private func width(_ view: some View) -> CGFloat {
        NSHostingView(rootView: view.font(.geek()).fixedSize()).fittingSize.width
    }

    @Test(arguments: [0, 1, 2, 4])
    func theCaretAddsNoWidthWhereverTheCursorIs(cursor: Int) {
        var line = LineEdit("devs")
        for _ in cursor..<4 { line.moveLeft() }
        #expect(line.cursor == cursor)
        let plain = width(Text("devs"))
        #expect(abs(width(CaretText(line: line)) - plain) < 0.5)
    }

    @Test func anEmptyLineIsOnlyItsCaret() {
        // No letter, but a caret one line high, a few points wide at most.
        let size = NSHostingView(rootView: CaretText(line: LineEdit()).font(.geek()).fixedSize()).fittingSize
        #expect(size.width < 1)
        #expect(size.height > 8)
    }

    @Test func theOldGlyphCaretWasALetterWide() {
        // The reason for `CaretText`: "▏" took a whole column.
        #expect(width(Text("dev▏s")) - width(Text("devs")) > 4)
    }
}
