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
