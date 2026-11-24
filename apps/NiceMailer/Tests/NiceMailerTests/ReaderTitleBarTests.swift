import AppKit
import SwiftUI
import Testing
@testable import NiceMailer

/// The title line of the reader (`ReaderTitleBar`): one line when the
/// whole subject fits beside the controls, two otherwise.
@MainActor
struct ReaderTitleBarTests {
    private let subject = "Ce soir, on parle d’École avec Édouard Philippe"

    /// The controls of a newsletter: unsubscribe, blocked pixels, switch.
    private func bar(_ subject: String) -> some View {
        ReaderTitleBar(subject: subject) {
            Text("unsubscribe")
            HStack { Image(systemName: "eye.slash"); Text("2") }
            Text(" text  html  images ")
        }
        .font(.interface())
    }

    /// Height the title line takes in a reader `width` points wide.
    ///
    /// Swift note: `NSHostingController` hosts a SwiftUI view in AppKit;
    /// `sizeThatFits(in:)` asks it the size it wants for a given width.
    private func height(_ view: some View, width: CGFloat) -> CGFloat {
        NSHostingController(rootView: view).sizeThatFits(in: CGSize(width: width, height: 10_000)).height
    }

    @Test func aWideReaderKeepsOneLine() {
        #expect(height(bar(subject), width: 1400) < scaled(30))
    }

    @Test func aNarrowReaderPutsTheControlsOnASecondLine() {
        let narrow = height(bar(subject), width: 600)
        #expect(narrow > scaled(40))
        // A short subject still fits on one line at the same width.
        #expect(height(bar("Devis"), width: 600) < scaled(30))
    }

    @Test func theChoiceFollowsTheLengthOfTheSubject() {
        // The width from which the subject fits whole beside the controls.
        let widths = stride(from: 300, through: 1600, by: 20).map { CGFloat($0) }
        let short = widths.first { height(bar("Devis"), width: $0) < scaled(30) }
        let long = widths.first { height(bar(subject), width: $0) < scaled(30) }
        #expect(short != nil && long != nil)
        #expect(long! > short!)
    }
}

/// The room around the subject of the reader follows the theme
/// (`layout.reader_title_padding`, `Look.readerTitleInsets`).
@MainActor
struct ReaderTitleInsetsTests {
    @Test func cleanGivesTheSubjectRoomAboveAndBelow() {
        // clean draws no line under the title: 12 above and below, at the
        // reference text size; on the sides, the column's margin alone.
        let theme = ThemeStore.builtins["clean"]!
        #expect(theme.layout.readerTitlePadding == 12)
        let pad = scaled(12)
        #expect(Look.readerTitleInsets(theme) == EdgeInsets(top: pad, leading: 12, bottom: pad, trailing: 12))
    }

    @Test func thePaddingNeverMovesTheSidesOfTheTitle() {
        // Whatever the padding (0 to 32), the subject starts and the
        // controls end at the column's margin, where the text does.
        var json = ThemeTests.themeJSON("clean")
        var layout = json["layout"] as! [String: Any]
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        for padding in [0, 12, 28, 32] {
            layout["reader_title_padding"] = padding
            json["layout"] = layout
            let theme = try! decoder.decode(ThemeView.self, from: JSONSerialization.data(withJSONObject: json))
            let insets = Look.readerTitleInsets(theme)
            #expect(insets.leading == Look.readerSideMargin, "\(padding)")
            #expect(insets.trailing == Look.readerSideMargin, "\(padding)")
            #expect(insets.top == scaled(CGFloat(padding)), "\(padding)")
        }
    }

    @Test func theTwoLinesOfTheTitleGrowApartWithThePadding() {
        // Half the padding: none in geek, 6 in clean, 16 at the largest.
        #expect(Look.readerTitleLineGap(ThemeStore.builtins["geek"]!) == 0)
        #expect(Look.readerTitleLineGap(ThemeStore.builtins["clean"]!) == scaled(6))
        var json = ThemeTests.themeJSON("clean")
        var layout = json["layout"] as! [String: Any]
        layout["reader_title_padding"] = 32
        json["layout"] = layout
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let wide = try! decoder.decode(ThemeView.self, from: JSONSerialization.data(withJSONObject: json))
        #expect(Look.readerTitleLineGap(wide) == scaled(16))
    }

    @Test func theButtonsOfTheTitleAreRoundedAsTheRowsForTheirSize() {
        // row_radius over row_height, brought to a 16 pt button.
        #expect(Look.titleControlRadius(ThemeStore.builtins["geek"]!) == 0)
        #expect(abs(Look.titleControlRadius(ThemeStore.builtins["clean"]!) - scaled(16.0 * 8 / 30)) < 0.001)
        #expect(abs(Look.titleControlRadius(ThemeStore.builtins["regular"]!) - scaled(4)) < 0.001)
    }

    @Test func aThemeWithLinesKeepsItsSideMarginsAlone() {
        // geek and regular: a line under the title already sets it apart.
        for name in ["geek", "regular"] {
            let insets = Look.readerTitleInsets(ThemeStore.builtins[name]!)
            #expect(insets == EdgeInsets(top: 0, leading: 12, bottom: 0, trailing: 12), "\(name)")
        }
    }
}

/// The title, the text reader and the HTML page share one column
/// (`Look.readerColumnSide`, `layout.reader_max_width`).
@MainActor
struct ReaderColumnTests {
    @Test func withoutABoundTheColumnKeepsTheMargin() {
        #expect(Look.readerColumnSide(width: 300, maxWidth: nil) == 12)
        #expect(Look.readerColumnSide(width: 2000, maxWidth: nil) == 12)
    }

    @Test func aBoundedColumnIsCenteredInAWiderReader() {
        // 1000 wide, a column of 560: 220 on each side.
        #expect(Look.readerColumnSide(width: 1000, maxWidth: 560) == 220)
        // Narrower than the column and its margins: the margin alone.
        #expect(Look.readerColumnSide(width: 580, maxWidth: 560) == 12)
        #expect(Look.readerColumnSide(width: 400, maxWidth: 560) == 12)
    }

    @Test func theTitleFrameGivesTheSameMarginAsTheText() {
        // ReaderTitleBar caps itself at maxWidth plus two margins, centered
        // in the reader: its margin, for any width, is the text's.
        for width in [300.0, 584, 600, 1000, 1700] as [CGFloat] {
            for bound in [nil, 320, 560, 1200] as [CGFloat?] {
                let capped = min(width, bound.map { $0 + 2 * Look.readerSideMargin } ?? width)
                let title = (width - capped) / 2 + Look.readerSideMargin
                #expect(abs(title - Look.readerColumnSide(width: width, maxWidth: bound)) < 0.001, "\(width) \(String(describing: bound))")
            }
        }
    }

    @Test func theHTMLPageGetsTheSameColumn() {
        // No bound: 12 px on the sides, the whole width.
        #expect(Look.readerColumnCSS(maxWidth: nil, vertical: 16) == "body { margin: 16px 0; padding: 0 12px; }")
        // A bound: the body at most the column and its padding, centered.
        let css = Look.readerColumnCSS(maxWidth: 560, vertical: 16)
        #expect(css.contains("box-sizing: border-box"))
        #expect(css.contains("max-width: 584.00px"))
        #expect(css.contains("margin: 16px auto"))
        #expect(css.contains("padding: 0 12px"))
        // A bound scaled with the text keeps its fraction.
        #expect(Look.readerColumnCSS(maxWidth: 560.5, vertical: 16).contains("max-width: 584.50px"))
    }
}
