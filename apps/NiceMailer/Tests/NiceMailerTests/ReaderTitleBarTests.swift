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
    @Test func cleanGivesTheSubjectRoomOnEverySide() {
        // clean draws no line under the title: 12 above and below, 12 more
        // than geek on the sides, at the reference text size.
        let theme = ThemeStore.builtins["clean"]!
        #expect(theme.layout.readerTitlePadding == 12)
        let pad = scaled(12)
        #expect(Look.readerTitleInsets(theme) == EdgeInsets(top: pad, leading: 12 + pad, bottom: pad, trailing: 12 + pad))
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
