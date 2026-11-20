import Foundation
import Testing
@testable import NiceMailer

/// The text sizes of the geek theme (`TextSize`, `docs/geek-ui.md`, "Text
/// size") and what derives from them.
@MainActor
struct TextSizeTests {
    @Test func startsAtTheDefaultSizes() {
        let t = TextSize()
        #expect(t.size == 16 && t.statusBarSize == 18)
        // The layout is written for 12 pt: 16 pt draws it a third bigger,
        // the status bar half as big again.
        #expect(abs(t.scale - 16.0 / 12.0) < 0.0001)
        #expect(abs(t.statusBarScale - 1.5) < 0.0001)
    }

    @Test func takesTheSizesOfTheFile() {
        let t = TextSize()
        t.configure(size: 18, statusBar: 11)
        #expect(t.size == 18 && t.statusBarSize == 11)
        #expect(abs(t.scale - 1.5) < 0.0001)
        #expect(t.statusBarScale < t.scale)
        // An older core gives none: the defaults.
        t.configure(size: nil, statusBar: nil)
        #expect(t.size == 16 && t.statusBarSize == 18)
        // Out of range (the core refuses it, but the app never trusts a
        // size it could not draw): clamped.
        t.configure(size: 99, statusBar: 1)
        #expect(t.size == 32 && t.statusBarSize == 9)
    }

    @Test func commandPlusAndMinusMoveBothSizesAndResetGoesBackToTheFile() {
        let t = TextSize()
        t.configure(size: 16, statusBar: 12)
        t.bigger(); t.bigger()
        #expect(t.size == 18 && t.statusBarSize == 14)
        t.smaller()
        #expect(t.size == 17 && t.statusBarSize == 13)
        t.reset()
        #expect(t.size == 16 && t.statusBarSize == 12)
    }

    @Test func staysWithinTheRangeAndAnswersTheOtherWayAtOnce() {
        let t = TextSize()
        for _ in 0..<40 { t.bigger() }
        #expect(t.size == 32 && t.statusBarSize == 32)
        // At the top, the first ⌘− shrinks: the step did not pile up. The
        // status bar (18 pt, 2 more than the window) reached 32 two steps
        // before the window, so the window shrinks first, the status bar
        // two steps later, and the difference of the file comes back.
        t.smaller()
        #expect(t.size == 31 && t.statusBarSize == 32)
        t.smaller(); t.smaller()
        #expect(t.size == 29 && t.statusBarSize == 31)
        for _ in 0..<40 { t.smaller() }
        #expect(t.size == 9 && t.statusBarSize == 9)
        // At the bottom, the window reached 9 first: the status bar grows
        // at the first ⌘+, the window two steps later.
        t.bigger()
        #expect(t.size == 9 && t.statusBarSize == 10)
        t.bigger(); t.bigger()
        #expect(t.size == 10 && t.statusBarSize == 12)
    }

    @Test func oneSizeAtItsBoundStillLetsTheOtherMove() {
        let t = TextSize()
        t.configure(size: 32, statusBar: 20)
        t.bigger()
        #expect(t.size == 32 && t.statusBarSize == 21)
    }

    @Test func anEditOfTheFileClearsTheStepAnUnchangedOneKeepsIt() {
        let t = TextSize()
        t.configure(size: 16, statusBar: 12)
        t.bigger()
        // `ConfigChanged` for another key: the same sizes, ⌘+ stays.
        t.configure(size: 16, statusBar: 12)
        #expect(t.size == 17)
        // The user wrote a new size: that one, exactly.
        t.configure(size: 20, statusBar: 12)
        #expect(t.size == 20 && t.statusBarSize == 12)
    }

    @Test func theMessageSaysBothSizesWhenTheyDiffer() {
        let t = TextSize()
        #expect(t.flash == L("Text size: %lld pt, status bar %lld pt", 16, 18))
        t.configure(size: 14, statusBar: 14)
        #expect(L(in: "en", "Text size: %lld pt", 14) == "Text size: 14 pt")
        #expect(t.flash == L("Text size: %lld pt", 14))
        t.configure(size: 18, statusBar: 12)
        #expect(t.flash == L("Text size: %lld pt, status bar %lld pt", 18, 12))
        #expect(L(in: "en", "Text size: %lld pt, status bar %lld pt", 18, 12) == "Text size: 18 pt, status bar 12 pt")
        #expect(L(in: "fr", "Text size: %lld pt, status bar %lld pt", 18, 12) == "Taille du texte : 18 pt, barre de statut 12 pt")
    }

    @Test func columnWidthsFollowTheScale() {
        let normal = RowLayout(width: 1000, scale: 1)
        let big = RowLayout(width: 1000, scale: 1.5)
        #expect(big.char > normal.char)
        #expect(abs(big.date - 8 * big.char) < 0.001)
    }
}
